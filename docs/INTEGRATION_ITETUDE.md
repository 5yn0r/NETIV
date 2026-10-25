# Intégration de NETIV dans ITEtude — Prompt Claude Code & cahier des charges

> **Comment utiliser ce document**
> - Copie la section **« Prompt à donner à Claude Code »** dans Claude Code, ouvert dans le dépôt concerné.
> - Place le reste du document (le cahier des charges) dans le dépôt, par exemple `docs/INTEGRATION_ITETUDE.md`, et indique son chemin dans le prompt.
> - Travaille en **deux sessions séparées** : une dans le dépôt NETIV (lots 1 à 3), une dans le dépôt ITEtude (lots 4 à 7).

---

## Prompt à donner à Claude Code

### Session 1 — dans le dépôt NETIV

```text
Tu travailles sur NETIV, un simulateur réseau pédagogique écrit en Rust (workspace de 10 crates,
édition 2024, rust 1.85). Le cahier des charges complet est dans docs/INTEGRATION_ITETUDE.md :
lis-le entièrement avant toute modification.

Contexte : NETIV va être intégré à ITEtude (itetude.com), une plateforme d'apprentissage IT
francophone en Next.js hébergée sur Vercel. Le simulateur doit tourner entièrement dans le
navigateur via WebAssembly, sans serveur. La CLI et le serveur `netiv serve` doivent continuer
à fonctionner exactement comme aujourd'hui.

Ta mission dans cette session : les lots 1, 2 et 3 du cahier des charges.
1. Préparer le moteur pour WASM (horloge injectée, stockage abstrait, parallélisme en feature,
   formats versionnés, garde-fou CI).
2. Extraire la logique métier de netiv-api dans une nouvelle crate netiv-service, puis faire de
   netiv-api une fine couche HTTP par-dessus.
3. Créer la crate netiv-wasm (wasm-bindgen) et un adaptateur pour l'interface web existante.

Règles :
- Commence par explorer le code et me proposer un plan détaillé lot par lot. Attends ma validation
  avant de modifier quoi que ce soit.
- Travaille sur une branche `feat/wasm-itetude`, un commit par étape logique. Ne pousse rien et
  ne touche jamais à `main`.
- Après chaque lot : `cargo fmt --check`, `cargo clippy --workspace -- -D warnings`,
  `cargo test --workspace` doivent passer, y compris les tests golden
  (crates/netiv-cli/tests/golden/). Aucun changement de comportement de la CLI ou de l'API HTTP.
- Respecte les lints du workspace : pas de `unsafe`, pas d'`unwrap()` hors tests, docs sur les
  éléments publics.
- Écris en français (code commenté, messages d'erreur, docs), comme le reste du projet.
- Si un point du cahier des charges te paraît faux ou risqué une fois dans le code, signale-le
  au lieu de le contourner.
```

### Session 2 — dans le dépôt ITEtude

```text
Tu travailles sur ITEtude (itetude.com), une plateforme d'apprentissage IT francophone en Next.js,
déployée sur Vercel, en production avec de vrais utilisateurs. Le cahier des charges complet est
dans docs/INTEGRATION_ITETUDE.md : lis-le entièrement avant toute modification.

Contexte : j'ai développé NETIV, un simulateur réseau en Rust, compilé en WebAssembly
(paquet produit par wasm-pack depuis le dépôt NETIV, crate netiv-wasm). Je veux l'intégrer à
ITEtude : un bac à sable /simulateur, des labs /labs/[slug] reliés aux parcours existants, la
progression sauvegardée dans le compte utilisateur, et un classement global.

Ta mission : les lots 4 à 7 du cahier des charges.

Règles :
- Commence par explorer le projet : version de Next.js (App Router ou Pages Router), système
  d'authentification, base de données et ORM, modèles existants (ressources, parcours,
  progression, utilisateurs), conventions de style et de composants. Fais-moi un résumé de
  l'existant, puis un plan détaillé. Attends ma validation avant de modifier quoi que ce soit.
- Travaille sur une branche `feat/netiv`, un commit par étape logique. Ne pousse rien, ne
  déploie rien, ne touche jamais à `main`.
- Toute migration de base de données doit être additive (nouvelles tables ou colonnes) :
  aucune suppression ni modification destructive des données existantes.
- Le simulateur ne doit pas alourdir le reste du site : le module WASM est chargé uniquement
  côté client, uniquement sur les pages qui l'utilisent.
- Respecte le design et les composants existants d'ITEtude. Tout le texte est en français.
- Le build Vercel et le lint doivent passer à chaque étape.
```

---

## Cahier des charges

### 1. Contexte

**ITEtude** (itetude.com) est une plateforme francophone qui organise l'apprentissage de l'informatique : ressources externes validées, parcours structurés, certifications, offres de stage, communauté, recherche IA. Elle est développée en Next.js et hébergée sur Vercel. Son public : étudiants, autodidactes et jeunes professionnels, en grande partie en Afrique de l'Ouest, souvent avec une connexion limitée (d'où le filtre « poids data » sur les ressources).

Son manque principal : tout pointe vers l'extérieur. Rien n'est pratiqué sur la plateforme elle-même.

**NETIV** (Network Côte d'Ivoire) est un laboratoire réseau interactif en Rust : construire, observer, casser, diagnostiquer. Version 0.4. Il contient :
- un moteur de simulation : ARP, ICMP, routage, switch avec VLAN, UDP, TCP minimal, HTTP simulé, pare-feu/ACL à état, DHCP (relais compris), DNS ;
- des outils d'observation : `trace` (parcours d'un paquet), `inspect` (trame couche par couche), `explain` (justification des décisions) ;
- 21 pannes réversibles, un diagnostic de cause racine, des indices à 4 niveaux ;
- 18 défis notés (débutant, intermédiaire, avancé), XP, niveaux, badges, parcours, mode enseignant ;
- un analyseur de sécurité, de la forensique sur captures, 3 cyber ranges ;
- un générateur de topologies et de défis, un « what-if », un score de santé ;
- une API HTTP (axum) et une interface web en JavaScript natif, servies par `netiv serve`.

**Objectif de l'intégration** : faire de NETIV le terrain de pratique d'ITEtude. Le simulateur tourne dans le navigateur de l'utilisateur via WebAssembly, sans serveur dédié, et sa progression est rattachée au compte ITEtude.

### 2. Principes directeurs

1. **Pas de fusion des dépôts.** NETIV reste un projet autonome. ITEtude le consomme comme une dépendance (paquet produit par `wasm-pack`).
2. **Zéro serveur de simulation.** Toute la simulation s'exécute côté client. Vercel ne sert que des fichiers statiques et les routes API d'ITEtude (sauvegarde de progression, classement).
3. **Aucune régression.** La CLI NETIV, `netiv serve`, l'API HTTP et les tests existants (dont les tests golden) continuent de fonctionner à l'identique. ITEtude en production n'est pas perturbé.
4. **Une seule source de vérité pour la logique.** La logique des réponses de l'API n'est écrite qu'une fois (crate `netiv-service`) et réutilisée par l'API HTTP et par WASM.
5. **Léger.** Cohérent avec le filtre « poids data » d'ITEtude : objectif de moins de 1,5 Mo pour le `.wasm` compressé (gzip ou brotli).
6. **Déterminisme conservé.** Une même topologie, une même graine et les mêmes commandes donnent le même résultat, en natif comme en WASM.

### 3. État actuel de NETIV (constaté dans le code)

| Élément | Constat | Conséquence pour WASM |
|---|---|---|
| `netiv-core` | Ne dépend que de `serde`, `serde_json`, `thiserror` | Compatible |
| Contenu pédagogique | Défis, topologies, parcours embarqués via `include_str!` (`netiv-education/src/builtin.rs`, `learn.rs`) | Compatible, rien à changer |
| Horloge | `Session::default()` utilise `system_seconds()` → `SystemTime::now()` (`netiv-cli/src/session.rs`) | Panique sur `wasm32-unknown-unknown` : à corriger |
| Horloge injectable | `set_clock`, `set_manual_clock`, `ManualClock` existent déjà | Base pour la correction |
| Fichiers | `std::fs` dans `netiv-cli/src/session.rs` et `session/lab.rs`, `netiv-education/src/profile.rs`, `netiv-security/src/forensics.rs` | Renvoie une erreur en WASM : à abstraire |
| Bac à sable | `Session::sandboxed()` interdit déjà l'accès aux fichiers utilisateur | Réutilisable |
| Parallélisme | `netiv-diagnostics/src/parallel.rs` : `std::thread::scope` ; retombe en séquentiel si `available_parallelism()` échoue | Fonctionne par accident : à rendre explicite |
| `netiv-api` | axum + tokio ; handlers qui mélangent logique et HTTP (`create_lab`, `exec`, `trace`, `transactions`, `journey`, `device`, `lab_health`, `lab_security`, `lab_docs`, `lab_what_if`, `get_topology`, `put_topology`, `validate_topology`, `autoconfig`, `put_positions`, `info`, `library`, `profile`, `leaderboard`…) | Logique à extraire |
| Interface web | `netiv-api/web/app.js` (≈2 300 lignes), tous les appels passent par une fonction `api(method, path, body)` | Point de bascule unique vers WASM |
| `netiv-bin`, `netiv-api` | Dépendent de tokio/axum | Restent natifs, exclus de la cible WASM |

### 4. Lots de travail — dépôt NETIV

#### Lot 1 — Préparer le moteur pour WebAssembly

**1.1 Horloge**
- La horloge système par défaut n'est compilée que hors WASM (`#[cfg(not(target_arch = "wasm32"))]`).
- En WASM, la session est construite sans horloge système ; l'horloge est toujours fournie de l'extérieur (côté JS : `Date.now()`).
- Aucun appel à `SystemTime` ou `Instant` ne doit rester dans les crates compilées pour WASM.

**1.2 Stockage abstrait**
- Créer un trait (par exemple `Storage`) couvrant les besoins actuels : lire, écrire, lister, renommer de façon atomique.
- Deux implémentations : `FsStorage` (comportement actuel, CLI et serveur) et `MemoryStorage` (web).
- Le profil élève, les labs `.netiv`, les captures `.nlv` et le classement local passent par ce trait.
- Ajouter `Profile::to_json()` et `Profile::from_json()` : côté ITEtude, le profil sera sauvegardé dans la base de données, pas dans un fichier.

**1.3 Parallélisme**
- Feature cargo `parallel`, activée par défaut, qui active l'exécution multi-thread de `par_map_with`.
- Sans la feature, exécution séquentielle, résultats identiques et dans le même ordre.

**1.4 Versionnement des formats**
- Ajouter un champ `schema_version` aux formats sérialisés destinés à être stockés à l'extérieur : topologie, lab `.netiv`, profil.
- Les fichiers existants sans ce champ restent lisibles (valeur par défaut = version actuelle).
- Documenter la règle de compatibilité dans `docs/`.

**1.5 Garde-fou CI**
- Nouveau job dans `.github/workflows/ci.yml` : `rustup target add wasm32-unknown-unknown` puis `cargo check --target wasm32-unknown-unknown --no-default-features` sur `netiv-core`, `netiv-topology`, `netiv-diagnostics`, `netiv-chaos`, `netiv-security`, `netiv-analysis`, `netiv-education`, et plus tard `netiv-service` et `netiv-wasm`.

**Critères d'acceptation du lot 1**
- Tous les tests existants passent, golden compris.
- Le job CI WASM passe.
- `netiv run`, `netiv serve`, `netiv validate`, `netiv generate`, `netiv docs`, `netiv bench` se comportent comme avant.

#### Lot 2 — Crate `netiv-service`

- Nouvelle crate sans dépendance HTTP ni tokio, compilable en WASM.
- Elle contient la logique aujourd'hui dans les handlers de `netiv-api` : gestion d'un ensemble de labs (création depuis une topologie intégrée, un YAML, un défi ou une génération), exécution d'une commande, traces, transactions, parcours d'un paquet, état d'un équipement, santé, analyse de sécurité, documentation, what-if, lecture/écriture/validation/autoconfiguration de topologie, positions, bibliothèque, profil, classement.
- Chaque opération est une fonction typée qui renvoie une struct `serde` ou une erreur métier (`ServiceError`), sans notion de code HTTP.
- Les règles de sécurité actuelles sont conservées : limites de taille de topologie (`check_size`), validation des noms (`safe_name`, `token`), interdiction de certaines actions pendant un défi (`not_during_challenge`).
- `netiv-api` est réécrit comme une fine couche : extraction des paramètres HTTP → appel à `netiv-service` → conversion en JSON et en code HTTP. Les middlewares (`host_guard`, `security_headers`) restent dans `netiv-api`.

**Critères d'acceptation du lot 2**
- `crates/netiv-api/tests/api.rs` passe sans modification des assertions (mêmes routes, mêmes JSON, mêmes codes).
- `netiv-service` compile pour `wasm32-unknown-unknown`.
- Tests unitaires dans `netiv-service` pour les opérations principales.

#### Lot 3 — Crate `netiv-wasm` et adaptateur web

**3.1 Crate `netiv-wasm`**
- `crate-type = ["cdylib", "rlib"]`, dépendances `wasm-bindgen`, `serde-wasm-bindgen` (ou JSON en chaîne), `js-sys` pour l'horloge.
- Expose une classe JS (par exemple `NetivEngine`) qui encapsule `netiv-service` en mémoire :
  - `new(options)` : graine, profil JSON optionnel ;
  - `request(method, path, body)` : même contrat que l'API HTTP actuelle (mêmes chemins, mêmes JSON), pour que l'interface existante fonctionne sans réécriture ;
  - `exportProfile()` / `importProfile(json)` ;
  - `exportLab(id)` / `importLab(text)` ;
  - `version()`.
- Les erreurs remontent en objets JS `{ status, error }` cohérents avec l'API HTTP.
- Profil de compilation release : `opt-level = "z"`, `lto = true`, `codegen-units = 1`, `panic = "abort"`, puis `wasm-opt -Oz`.
- Hook de panique (`console_error_panic_hook`) en mode développement seulement.

**3.2 Adaptateur de l'interface web**
- Dans `app.js`, la fonction `api(method, path, body)` choisit son transport : `fetch` (mode serveur, comportement actuel) ou `NetivEngine.request` (mode WASM).
- Le mode est choisi à l'initialisation (présence du module WASM ou paramètre explicite).
- Les fonctions sans équivalent web (lecture de fichiers du disque) affichent un message clair au lieu d'échouer.

**3.3 Paquet et démo**
- `wasm-pack build crates/netiv-wasm --target web --release` produit le paquet `pkg/`.
- Une page de démonstration statique (`crates/netiv-wasm/demo/` ou équivalent) charge le module et l'interface web, sans serveur NETIV, servie par un simple serveur de fichiers.
- Documentation dans `docs/wasm.md` : compilation, API JS, taille obtenue, limites.

**Critères d'acceptation du lot 3**
- Dans la démo, on peut : charger la topologie `mvp`, faire `ping PC1 192.168.2.10`, voir le 1er ping à 16 ms et le 2ᵉ à 8 ms comme en natif, afficher `trace`, `inspect`, `explain`, lancer et réussir un défi débutant, éditer une topologie.
- Les sorties d'une même suite de commandes sont identiques en natif et en WASM (test comparatif automatisé, par exemple sur le scénario golden).
- Taille du `.wasm` compressé mesurée et documentée ; si elle dépasse 1,5 Mo, proposer des features pour exclure des modules lourds (sécurité/forensique, générateurs).

### 5. Lots de travail — dépôt ITEtude

#### Lot 4 — Intégration technique du paquet

- Le paquet `netiv-wasm` est ajouté à ITEtude par l'une de ces voies (à choisir après exploration, la première est recommandée) :
  1. paquet npm publié ou release GitHub du dépôt NETIV, version figée ;
  2. fichiers `pkg/` copiés dans un dossier dédié, avec un script de mise à jour et la version notée.
- Le `.wasm` est servi comme fichier statique, avec le bon type MIME (`application/wasm`) et un cache long (nom de fichier versionné).
- Chargement uniquement côté client (`dynamic import`, `ssr: false` ou équivalent), uniquement sur les pages simulateur et labs.
- Un composant React (par exemple `<NetivSimulator />`) encapsule le moteur : initialisation, états de chargement et d'erreur, nettoyage au démontage.
- Indicateur de chargement avec la taille à télécharger, cohérent avec le badge « poids data » d'ITEtude.

#### Lot 5 — Pages et parcours utilisateur

**5.1 `/simulateur` — bac à sable**
- Accessible sans compte (sans sauvegarde) ; avec compte, sauvegarde des topologies.
- Choix d'une topologie de départ (topologies intégrées à NETIV) ou topologie vide.
- Terminal de commandes NETIV + vue topologie + panneaux trace / inspect / explain.
- Export et import d'un lab (`.netiv`).

**5.2 `/labs` et `/labs/[slug]`**
- Catalogue des labs : les 18 défis de NETIV et les 3 cyber ranges, classés par niveau (débutant, intermédiaire, avancé) et par domaine (Réseau, Cybersécurité), avec les mêmes filtres que la page Ressources.
- Page d'un lab : objectif, contexte, durée estimée, simulateur intégré, indices, soumission de la réponse, score détaillé (diagnostic, correction, temps, pénalité d'indices).
- Pages indexables (titre, description, métadonnées) pour le référencement ; le simulateur lui-même se charge après.

**5.3 Liens avec l'existant**
- Un lab peut être ajouté comme étape d'un parcours ITEtude ; sa réussite valide l'étape dans la progression existante.
- Bouton « Pratiquer dans NETIV » sur les ressources du domaine Réseau (et Cybersécurité quand c'est pertinent), qui pointe vers un lab ou le bac à sable avec une topologie adaptée.
- Entrée « Simulateur » ou « Labs » dans la navigation principale, avec le badge « Nouveau » utilisé par le site.

#### Lot 6 — Données, progression et classement

**Modèle de données (additif uniquement)** — à adapter à l'ORM existant :
- `netiv_profiles` : utilisateur, profil NETIV en JSON, `schema_version`, date de mise à jour.
- `netiv_labs` : utilisateur, titre, contenu du lab (texte `.netiv` ou JSON), `schema_version`, visibilité (privé / lien partageable), dates.
- `netiv_attempts` : utilisateur, identifiant du défi, score, détail du score, durée, indices utilisés, date.

**Règles**
- Le profil est chargé dans le moteur à l'ouverture (`importProfile`) et sauvegardé après chaque défi terminé (`exportProfile`), via des routes API d'ITEtude authentifiées.
- **Le score envoyé par le client n'est pas digne de confiance** : le classement global est indicatif. Mesures minimales : bornes de cohérence (score maximal, durée minimale réaliste), limite de fréquence par utilisateur, possibilité pour l'administrateur de retirer une entrée. Documenter cette limite.
- Validation de taille et de format sur tout contenu envoyé (lab, profil).
- Classement global : par XP total et par défi, visible sur `/labs`.
- Labs partagés : un lien en lecture seule, que le visiteur peut dupliquer dans son propre espace.

#### Lot 7 — Qualité, sécurité, performance

- **Performance** : aucune page existante ne doit charger le WASM ; mesurer le poids des pages simulateur/labs ; vérifier le fonctionnement sur mobile et sur connexion lente (throttling 3G).
- **Sécurité** : en-têtes adaptés au WASM (CSP autorisant `'wasm-unsafe-eval'` sur les pages concernées uniquement) ; aucune donnée utilisateur dans les URL ; routes API protégées par l'authentification existante.
- **Accessibilité** : terminal utilisable au clavier, contrastes conformes au thème du site.
- **Tests** : tests de bout en bout sur le parcours « ouvrir un lab → réussir un défi → progression enregistrée » ; test que le build Vercel passe.
- **Suivi** : événements simples (lab commencé, lab réussi, abandon) si le site dispose déjà d'un outil d'analyse.

### 6. Hors périmètre (pour plus tard)

- Portage complet de l'interface NETIV en composants React (l'interface existante est d'abord réutilisée).
- Mode enseignant sur ITEtude (création de classes, suivi des élèves).
- Labs créés et soumis par la communauté via la page « Participer ».
- Validation des scores côté serveur (rejouer la session dans une fonction serverless ou un service Rust natif).
- Défis multi-joueurs ou temps réel.

### 7. Ordre de livraison et jalons

| Jalon | Contenu | Résultat visible |
|---|---|---|
| J1 | Lot 1 | NETIV compile en WASM (CI verte), aucune régression |
| J2 | Lot 2 | `netiv-service` en place, `netiv-api` allégé, tests API inchangés |
| J3 | Lot 3 | Démo NETIV dans le navigateur, sans serveur |
| J4 | Lots 4 + 5.1 | `/simulateur` en ligne sur ITEtude (branche de prévisualisation Vercel) |
| J5 | Lots 5.2 + 6 | Labs, progression sauvegardée, classement |
| J6 | Lots 5.3 + 7 | Liens avec les parcours et ressources, recette finale, mise en production |

Chaque jalon se termine par une revue humaine avant le suivant. La mise en production d'ITEtude n'a lieu qu'après validation sur un déploiement de prévisualisation Vercel.

### 8. Définition de « terminé »

- Toutes les vérifications automatiques passent (fmt, clippy, tests Rust, job WASM, lint et build Next.js).
- Les critères d'acceptation de chaque lot sont vérifiés. Ne crée aucune branche, ne fait aucun commit: laisse moi fait moi meme. 
La documentation est à jour : `docs/wasm.md` côté NETIV, une page d'explication côté ITEtude pour l'ajout de nouveaux labs.
- Aucune donnée de production n'a été modifiée en dehors des migrations additives prévues.