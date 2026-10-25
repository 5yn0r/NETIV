# Architecture

Vue d'ensemble pour qui veut lire ou modifier le code. La référence détaillée (types, choix,
écarts) est la [spécification d'architecture](../SPEC_ARCHITECTURE.md).

## Les crates

```
                      netiv-core          moteur : événements, protocoles, trace, rendu texte
                          │
                    netiv-topology        YAML ⇄ Simulation, bundles .netiv, générateur
          ┌───────────┬───┴────────┬──────────────┐
  netiv-diagnostics  netiv-chaos  netiv-security  netiv-analysis
  sondes, cause      pannes       analyseur,      accessibilité, what-if,
  racine, indices    seedées      forensique      santé, documentation
          └───────────┴───┬────────┴──────────────┘
                    netiv-education       défis, scores, profils, cours, cyber ranges, tuteur
                          │
                      netiv-cli           Session : interprète des commandes (bibliothèque)
                          │
                    netiv-service         logique de l'API sans HTTP : labs, limites, JSON
                          │
               netiv-api ─┴─ netiv-bin    API HTTP + web embarqué │ binaire « netiv »
```

Règles :

- `netiv-core` ne dépend d'aucune interface, d'aucun runtime asynchrone et d'aucune entrée/sortie.
  Il ne dépend que de `serde`.
- Tokio et axum n'apparaissent que dans `netiv-api` (et `netiv-bin`, qui le lance).
- La logique des réponses de l'API (création des labs, limites, verrous pendant un défi, JSON)
  est écrite **une fois**, dans `netiv-service`. `netiv-api` n'y ajoute que le HTTP : extraction
  des paramètres, codes de retour, en-têtes de sécurité, contrôle de l'hôte. `Service::request`
  rejoue les mêmes routes sans HTTP pour le navigateur (WebAssembly).
- Les fichiers (profils, labs, captures) passent par le trait `netiv_cli::Storage` : le disque
  en CLI et en serveur, la mémoire dans le navigateur.
- La CLI et l'API partagent la **même** `Session` : une commande se comporte pareil partout.

## Le moteur en une page

```
          ┌──────────────── EventQueue (date, n° d'ordre) ────────────────┐
          │  FrameArrival · Timer · Command · Fault                       │
          └───────────────┬───────────────────────────────────────────────┘
                          │ step() : l'événement le plus ancien
                          ▼
   Simulation ──► équipement (IpNode / Switch) ──► Effects { Send, SetTimer, Trace }
        ▲                                                    │
        └────────── applique : programme les arrivées, ──────┘
                    ajoute à la trace (TraceLog)
```

- **Temps virtuel** en microsecondes (`SimTime`). Rien ne dépend de l'horloge réelle.
- **Déterminisme** : file ordonnée par (date, numéro d'ordre) et générateur aléatoire maison
  (`SimRng`, SplitMix64) initialisé par la graine du réseau. Même réseau, mêmes commandes →
  même trace, octet pour octet. Les tests le vérifient pour chaque module.
- **Les équipements ne touchent pas au monde** : ils reçoivent un contexte en lecture seule et
  rendent des `Effects`. Seule la `Simulation` modifie l'état global.
- **`IpNode`** est partagé par les hôtes, serveurs, routeurs et pare-feu (ARP, ICMP, routage, UDP,
  TCP simplifié, services, ACL à état). **`Switch`** gère l'apprentissage MAC par VLAN, les ports
  d'accès et les trunks.

## La trace causale

Tout ce qui se passe est consigné dans `TraceLog` : `TraceEvent { seq, time, device, trace_id,
caused_by, kind }`.

- `trace_id` regroupe une **transaction entière** : requête, réponse, ARP déclenché, erreur ICMP.
  `trace 3` montre donc l'aller-retour complet. Un index permet de retrouver ses événements
  sans parcourir la trace.
- `caused_by` relie chaque événement à celui qui l'a provoqué (une réception → l'envoi d'origine).
- Le Packet Journey (`trace`), le Packet Microscope (`inspect`), l'Explain Engine (`explain`),
  le diagnostic et la forensique sont des **vues** sur cette trace (`netiv-core::render`,
  `netiv-diagnostics`, `netiv-security::forensics`).
- Chaque décision de routage enregistre une `RouteDecision` complète : toutes les routes
  candidates, les routes inactives (prochain saut injoignable) et le choix. C'est ce qui permet
  d'expliquer, et non seulement de constater.

## Sondes et analyses

Le diagnostic, le what-if, la santé, l'analyse de sécurité et la vérification des défis posent
tous des questions du type « ce flux passe-t-il ? ». Ils y répondent avec des **sondes** :

1. `fresh_clone()` : copie de la configuration, sans trace, sans file d'attente, caches vidés ;
2. un ping, un `curl` ou un datagramme dans cette copie ;
3. lecture de la trace de la copie.

La simulation de l'utilisateur n'est donc jamais modifiée par une analyse. Les sondes étant
indépendantes, `netiv-diagnostics::parallel::par_map` les répartit sur les cœurs en conservant
l'ordre des résultats.

## Pannes réversibles

Une `Fault` (câble coupé, mauvaise passerelle, règle ACL…) mémorise l'état qu'elle remplace :
`chaos undo` restaure exactement la configuration d'avant. Les pannes peuvent être programmées
dans le temps (`Event::Fault`), ce qu'utilisent les cyber ranges.

## Interface web

`netiv-api` sert trois fichiers embarqués dans le binaire (`include_str!`) : `index.html`,
`style.css`, `app.js`. C'est du JavaScript sans framework ni étape de compilation. La page
n'appelle que l'API JSON décrite dans [api.md](api.md). Le schéma est un SVG : disposition
radiale calculée côté client, positions et zoom mémorisés dans le navigateur.

## Où ajouter…

| … | Où |
|---|---|
| un protocole ou un comportement d'équipement | `netiv-core/src/ip_node.rs` ou `switch.rs`, un `TraceKind`, son rendu dans `render.rs` |
| une commande | `netiv-cli/src/session.rs` (aiguillage) et le module de `session/` concerné, `help`, [reference-commandes.md](reference-commandes.md) |
| une panne du catalogue | `netiv-chaos/src/catalog.rs` (+ cause et indices dans `netiv-diagnostics`) |
| un contrôle de sécurité | `netiv-security/src/analyzer.rs` |
| un défi livré | `netiv-education/content/challenges/…` (testé automatiquement) |
| un endpoint | l'opération dans `netiv-service/src/lib.rs` (+ `snapshot.rs` pour le JSON), sa route dans `netiv-service/src/request.rs` et dans `netiv-api/src/lib.rs` |
