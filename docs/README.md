# Documentation NETIV

| Document | Pour qui | Contenu |
|---|---|---|
| [Guide utilisateur](guide-utilisateur.md) | élèves, formateurs | Prise en main (terminal et web), tutoriel « casser puis réparer », défis, cyber range |
| [Référence des commandes](reference-commandes.md) | tous | Chaque commande de la session, avec syntaxe et exemples |
| [Format des topologies](format-topologie.md) | auteurs de labs | YAML complet : équipements, interfaces, VLAN, services, ACL ; fichiers `.netiv` |
| [Écrire un défi, un cyber range, un cours](format-defis.md) | formateurs | YAML des défis, scénarios de cyber range, cours (mode enseignant) |
| [API HTTP](api.md) | intégrateurs | Endpoints de `netiv serve`, exemples, garde-fous de sécurité |
| [Architecture](architecture.md) | développeurs | Crates, cycle d'un événement, trace causale, sondes |
| [Performance](performance.md) | développeurs | Modèle de coût, `netiv bench`, résultats et limites |
| [Contribuer](contribuer.md) | développeurs | Outils, tests, conventions du code |

Références de conception : [cahier des charges](../Cahier_de_charge.md) ·
[spécification d'architecture](../SPEC_ARCHITECTURE.md) (avec les écarts et les formules en annexes).

La documentation du code (rustdoc) se génère avec :

```bash
cargo doc --workspace --no-deps --open
```
