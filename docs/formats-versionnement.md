# Versionnement des formats

Certains fichiers de NETIV sont conservés en dehors de NETIV : sur le disque de l'élève, dans
une bibliothèque de labs, ou dans la base de données d'une plateforme qui intègre le simulateur
(ITEtude). Chacun de ces formats porte un numéro de version, pour qu'une version future de
NETIV sache ce qu'elle lit.

## Les formats et leur champ de version

| Format | Fichier | Champ | Version actuelle | Constante |
|---|---|---|---|---|
| Topologie | `.yaml` | `schema_version` | 1 | `netiv_topology::TOPOLOGY_SCHEMA_VERSION` |
| Lab partageable | `.netiv` | `format_version` | 1 | `netiv_topology::NETIV_FORMAT_VERSION` |
| Profil d'élève | `.json` | `schema_version` | 1 | `netiv_education::profile::PROFILE_SCHEMA_VERSION` |
| Capture | `.nlv` | `format_version` | 1 | `netiv_security::forensics::CAPTURE_FORMAT_VERSION` |

Un lab `.netiv` contient une topologie : sa version est celle du lab (`format_version`), et la
topologie qu'il contient suit la même règle que les topologies seules.

## Règle de compatibilité

1. **Champ absent = version 1.** Les fichiers écrits avant l'apparition du champ restent lisibles
   tels quels. Aucune conversion n'est nécessaire.
2. **NETIV lit toutes les versions inférieures ou égales à la sienne.** Une nouvelle version du
   format doit savoir lire les précédentes, éventuellement en convertissant à la lecture.
3. **Une version plus récente est refusée** avec un message clair (« format version N non
   supporté… »), jamais lue à moitié. Le fichier n'est pas modifié.
4. **On ne change de version que pour un changement incompatible** : champ renommé ou supprimé,
   sens d'un champ modifié. Ajouter un champ facultatif (avec une valeur par défaut) ne change
   pas la version.
5. **Topologies : `schema_version` n'est écrit que s'il diffère de 1.** Les topologies de
   version 1 exportées (`lab export`, `topology generate`, `docs yaml`, API) restent identiques,
   octet pour octet, à celles des versions précédentes de NETIV. Le profil, lui, écrit toujours
   son `schema_version`.

## Pour un intégrateur

- Stocker le texte tel que NETIV le produit (`Profile::to_json`, `Bundle::to_yaml`,
  `Capture::to_json`) et le rendre tel quel (`Profile::from_json`, `load_any`,
  `Capture::from_json`).
- Garder aussi la version à côté du contenu (par exemple une colonne `schema_version`) permet de
  repérer les fichiers à faire migrer sans les ouvrir.
- Avant de mettre à jour NETIV vers une version qui change un format, vérifier dans les notes de
  version comment les anciens fichiers sont convertis.
