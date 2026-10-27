//! Stockage des fichiers manipulés par une session : profils, labs `.netiv`,
//! captures `.nlv`, exports.
//!
//! La session ne touche jamais au disque directement : elle passe par un
//! [`Storage`]. La CLI et `netiv serve` utilisent [`FsStorage`] (le disque,
//! comme avant) ; le navigateur (WebAssembly) utilise [`MemoryStorage`].

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use netiv_education::Profile;
use netiv_education::profile::ProfileError;

/// Accès aux fichiers d'une session.
///
/// Les chemins sont ceux que donne l'utilisateur ou la configuration ; une
/// implémentation peut les interpréter comme elle veut (disque, mémoire...).
/// Les erreurs sont des [`io::Error`] : un fichier absent doit donner
/// [`io::ErrorKind::NotFound`].
pub trait Storage: Send {
    /// Lit un fichier texte.
    fn read(&self, path: &Path) -> io::Result<String>;

    /// Écrit (crée ou remplace) un fichier texte.
    fn write(&self, path: &Path, text: &str) -> io::Result<()>;

    /// Renomme un fichier (remplace la destination si elle existe).
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()>;

    /// Chemins des entrées directement contenues dans `dir` (ordre quelconque).
    fn list(&self, dir: &Path) -> io::Result<Vec<PathBuf>>;

    /// Un fichier existe-t-il à ce chemin ?
    fn is_file(&self, path: &Path) -> bool;

    /// Quelque chose (fichier ou dossier) existe-t-il à ce chemin ?
    fn exists(&self, path: &Path) -> bool;

    /// Crée un dossier et ses parents (sans effet s'il existe).
    fn create_dir_all(&self, dir: &Path) -> io::Result<()>;

    /// Un autre accès aux **mêmes** fichiers (pour une autre session).
    fn share(&self) -> Box<dyn Storage>;

    /// Écriture atomique : le dossier parent est créé, le texte est écrit
    /// dans un fichier temporaire, puis renommé. Un lecteur voit l'ancien
    /// contenu ou le nouveau, jamais un fichier à moitié écrit.
    fn write_atomic(&self, path: &Path, text: &str) -> io::Result<()> {
        if let Some(dir) = path.parent() {
            if !dir.as_os_str().is_empty() {
                self.create_dir_all(dir)?;
            }
        }
        let mut tmp = path.as_os_str().to_owned();
        tmp.push(".tmp");
        let tmp = PathBuf::from(tmp);
        self.write(&tmp, text)?;
        self.rename(&tmp, path)
    }
}

/// Le disque de la machine (CLI, `netiv serve`).
#[derive(Debug, Clone, Copy, Default)]
pub struct FsStorage;

impl Storage for FsStorage {
    fn read(&self, path: &Path) -> io::Result<String> {
        std::fs::read_to_string(path)
    }

    fn write(&self, path: &Path, text: &str) -> io::Result<()> {
        std::fs::write(path, text)
    }

    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        std::fs::rename(from, to)
    }

    fn list(&self, dir: &Path) -> io::Result<Vec<PathBuf>> {
        Ok(std::fs::read_dir(dir)?
            .flatten()
            .map(|e| e.path())
            .collect())
    }

    fn is_file(&self, path: &Path) -> bool {
        path.is_file()
    }

    fn exists(&self, path: &Path) -> bool {
        path.exists()
    }

    fn create_dir_all(&self, dir: &Path) -> io::Result<()> {
        std::fs::create_dir_all(dir)
    }

    fn share(&self) -> Box<dyn Storage> {
        Box::new(FsStorage)
    }
}

/// Fichiers gardés en mémoire (navigateur, tests). Les clones partagent le
/// même contenu : l'hôte peut garder un clone pour relire ce que la session
/// a écrit (par exemple le profil, pour le sauvegarder ailleurs).
#[derive(Debug, Clone, Default)]
pub struct MemoryStorage(Arc<Mutex<BTreeMap<PathBuf, String>>>);

impl MemoryStorage {
    /// Stockage vide.
    pub fn new() -> Self {
        Self::default()
    }

    fn files(&self) -> std::sync::MutexGuard<'_, BTreeMap<PathBuf, String>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Les chemins de tous les fichiers, dans l'ordre.
    pub fn paths(&self) -> Vec<PathBuf> {
        self.files().keys().cloned().collect()
    }

    /// Retire un fichier ; rend son contenu s'il existait.
    pub fn remove(&self, path: &Path) -> Option<String> {
        self.files().remove(path)
    }
}

fn not_found(path: &Path) -> io::Error {
    io::Error::new(
        io::ErrorKind::NotFound,
        format!("{} : fichier introuvable", path.display()),
    )
}

impl Storage for MemoryStorage {
    fn read(&self, path: &Path) -> io::Result<String> {
        self.files()
            .get(path)
            .cloned()
            .ok_or_else(|| not_found(path))
    }

    fn write(&self, path: &Path, text: &str) -> io::Result<()> {
        self.files().insert(path.to_path_buf(), text.to_string());
        Ok(())
    }

    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        let mut files = self.files();
        let text = files.remove(from).ok_or_else(|| not_found(from))?;
        files.insert(to.to_path_buf(), text);
        Ok(())
    }

    fn list(&self, dir: &Path) -> io::Result<Vec<PathBuf>> {
        Ok(self
            .files()
            .keys()
            .filter(|p| p.parent() == Some(dir))
            .cloned()
            .collect())
    }

    fn is_file(&self, path: &Path) -> bool {
        self.files().contains_key(path)
    }

    fn exists(&self, path: &Path) -> bool {
        self.files().keys().any(|p| p.starts_with(path))
    }

    fn create_dir_all(&self, _dir: &Path) -> io::Result<()> {
        // Les dossiers n'existent qu'à travers les fichiers qu'ils contiennent.
        Ok(())
    }

    fn share(&self) -> Box<dyn Storage> {
        Box::new(self.clone())
    }
}

/// Lit un profil dans `storage` ; un fichier absent donne un profil neuf.
pub fn load_profile(
    storage: &dyn Storage,
    path: &Path,
    name: &str,
) -> Result<Profile, ProfileError> {
    match storage.read(path) {
        Ok(text) => Profile::from_json(&text, &path.display().to_string()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Profile::new(name)),
        Err(e) => Err(ProfileError::Io {
            path: path.display().to_string(),
            reason: e.to_string(),
        }),
    }
}

/// Stockage par défaut : le disque en natif, la mémoire en WebAssembly (où
/// le système de fichiers n'existe pas).
pub fn default_storage() -> Box<dyn Storage> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        Box::new(FsStorage)
    }
    #[cfg(target_arch = "wasm32")]
    {
        Box::new(MemoryStorage::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_storage_reads_back_writes_lists_and_renames() {
        let s = MemoryStorage::new();
        let p = Path::new("profils/alice.json");
        assert_eq!(s.read(p).unwrap_err().kind(), io::ErrorKind::NotFound);
        s.write_atomic(p, "{}").unwrap();
        assert_eq!(s.read(p).unwrap(), "{}");
        assert!(s.is_file(p));
        assert!(s.exists(Path::new("profils")));
        assert!(!s.is_file(Path::new("profils/alice.json.tmp")));
        s.write(Path::new("profils/bob.json"), "x").unwrap();
        s.write(Path::new("autre/c.json"), "y").unwrap();
        let mut listed = s.list(Path::new("profils")).unwrap();
        listed.sort();
        assert_eq!(
            listed,
            vec![
                PathBuf::from("profils/alice.json"),
                PathBuf::from("profils/bob.json")
            ]
        );
        let clone = s.clone();
        clone.rename(p, Path::new("profils/a.json")).unwrap();
        assert!(!s.is_file(p));
        assert_eq!(s.read(Path::new("profils/a.json")).unwrap(), "{}");
    }

    #[test]
    fn fs_storage_writes_atomically() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("sous/dossier/p.json");
        FsStorage.write_atomic(&p, "ok").unwrap();
        assert_eq!(FsStorage.read(&p).unwrap(), "ok");
        assert!(!dir.path().join("sous/dossier/p.json.tmp").exists());
        assert_eq!(FsStorage.list(p.parent().unwrap()).unwrap(), vec![p]);
    }
}
