Oui man. **NETIV** est un bien meilleur nom : court, mémorisable, et l'idée de *Network Côte d’Ivoire* donne une identité au projet sans limiter son utilisation à la Côte d’Ivoire.

Je te propose de figer maintenant le projet comme un **vrai cahier des charges**, pas juste une liste d'idées. L'objectif est que ce document puisse ensuite servir de référence pendant tout le développement.

# CAHIER DES CHARGES — NETIV

### **NETIV — Network Côte d’Ivoire**

**Laboratoire réseau interactif, pédagogique et de cybersécurité**

**Version :** 1.0
**Langage principal :** Rust
**Cible initiale :** Linux
**Type :** Open Source / Educational / Network Simulation
**Statut :** Projet en conception

---

# 1. Présentation du projet

## 1.1. Contexte

L'apprentissage des réseaux informatiques repose souvent sur des outils de simulation ou de virtualisation tels que Packet Tracer, GNS3 ou EVE-NG.

Ces outils sont extrêmement utiles pour construire des topologies et configurer des équipements, mais NETIV poursuit une approche différente :

> **Ne pas seulement montrer qu'un réseau fonctionne, mais permettre de comprendre pourquoi il fonctionne, pourquoi il tombe en panne et comment le diagnostiquer.**

NETIV sera donc un **laboratoire réseau interactif** permettant de :

* créer des infrastructures réseau virtuelles ;
* simuler les communications ;
* observer les paquets ;
* comprendre leur parcours ;
* provoquer des pannes ;
* diagnostiquer les problèmes ;
* analyser les flux ;
* réaliser des exercices ;
* apprendre les réseaux et la sécurité ;
* créer des challenges pédagogiques.

---

# 2. Vision

La vision de NETIV est de devenir une plateforme permettant à un étudiant de passer progressivement de :

```text
"Je connais la théorie"
          ↓
"Je vois ce qui se passe"
          ↓
"Je comprends ce qui se passe"
          ↓
"Je peux diagnostiquer"
          ↓
"Je peux sécuriser"
          ↓
"Je peux concevoir"
```

NETIV ne doit donc pas être uniquement un simulateur.

Il doit devenir un :

> **Network Learning & Troubleshooting Environment**

---

# 3. Problématique

Un étudiant peut connaître :

* OSI ;
* TCP/IP ;
* IPv4 ;
* subnetting ;
* ARP ;
* VLAN ;
* routage ;
* DNS ;
* DHCP ;

mais avoir beaucoup de difficultés lorsqu'on lui demande :

> « Pourquoi cette machine ne communique-t-elle pas avec cette autre ? »

NETIV doit transformer cette difficulté en expérience pratique.

---

# 4. Objectifs

## 4.1. Objectif général

Développer en Rust un environnement de simulation réseau permettant de **construire, observer, perturber, diagnostiquer et sécuriser des réseaux virtuels**.

## 4.2. Objectifs spécifiques

NETIV devra permettre de :

1. créer des topologies réseau ;
2. créer des hôtes, switches et routeurs virtuels ;
3. attribuer des adresses IP ;
4. configurer des interfaces ;
5. simuler Ethernet ;
6. simuler ARP ;
7. simuler IPv4 ;
8. simuler ICMP ;
9. simuler TCP/UDP ;
10. implémenter le routage ;
11. simuler DHCP ;
12. simuler DNS ;
13. simuler VLAN ;
14. observer les paquets ;
15. suivre leur parcours ;
16. provoquer des pannes ;
17. diagnostiquer ces pannes ;
18. créer des exercices ;
19. créer des challenges ;
20. évaluer les utilisateurs ;
21. analyser la sécurité d'une topologie.

---

# 5. Public cible

### Étudiants

* Licence 1
* Licence 2
* Licence 3
* Master
* étudiants ingénieurs

### Enseignants

Pour :

* travaux pratiques ;
* examens ;
* démonstrations ;
* exercices ;
* challenges.

### Professionnels débutants

Pour pratiquer :

* administration réseau ;
* troubleshooting ;
* architecture réseau ;
* sécurité réseau.

### Clubs informatiques / cybersécurité

Pour organiser :

* challenges ;
* compétitions ;
* formations ;
* simulations.

---

# 6. Fonctionnalités principales

## MODULE A — Topology Builder

L'utilisateur pourra créer une infrastructure.

Exemple :

```text
             R1
            /  \
          SW1  SW2
         /  \   / \
       PC1 PC2 PC3 PC4
```

### Équipements

Version initiale :

* PC ;
* serveur ;
* switch ;
* routeur ;
* firewall.

Plus tard :

* access point ;
* IDS/IPS ;
* load balancer ;
* cloud ;
* ISP.

---

# 7. Configuration des équipements

Chaque équipement possède :

```text
hostname
interfaces
MAC addresses
IP addresses
routing table
ARP table
services
configuration
logs
```

Exemple :

```text
Router R1

Interface eth0
IP: 192.168.1.1/24
Status: UP

Interface eth1
IP: 10.0.0.1/30
Status: UP
```

---

# 8. Network Simulation Engine

C'est le **cœur de NETIV**.

Il sera responsable de simuler les communications.

Architecture conceptuelle :

```text
              NETIV ENGINE
                   │
        ┌──────────┼──────────┐
        │          │          │
     Ethernet     ARP        IPv4
        │          │          │
       VLAN       ICMP      Routing
                              │
                         TCP / UDP
                              │
                    DHCP / DNS / HTTP
```

Le moteur devra gérer :

* transmission ;
* réception ;
* encapsulation ;
* désencapsulation ;
* tables ;
* interfaces ;
* routage ;
* TTL ;
* broadcast ;
* unicast ;
* erreurs.

---

# 9. Packet Engine

Chaque paquet sera représenté comme un objet simulé.

Exemple conceptuel :

```text
Ethernet
 ├── Source MAC
 ├── Destination MAC
 └── Payload
       │
       └── IPv4
            ├── Source IP
            ├── Destination IP
            ├── TTL
            └── Payload
                  │
                  └── ICMP
```

NETIV devra pouvoir afficher les différentes couches.

---

# 10. Packet Microscope

Fonctionnalité majeure.

L'utilisateur pourra sélectionner un paquet :

```bash
netiv packet inspect 42
```

NETIV affichera :

```text
PACKET #42

Ethernet
Source      : AA:BB:CC:DD:EE:01
Destination : AA:BB:CC:DD:EE:FF

IPv4
Source      : 192.168.1.10
Destination : 192.168.2.10
TTL         : 64
Protocol    : ICMP

ICMP
Type        : Echo Request
```

---

# 11. Packet Journey

NETIV montrera le parcours complet :

```text
PC1
 │
 │ Echo Request
 ▼
SW1
 │
 ▼
R1
 │ TTL 64 → 63
 ▼
R2
 │ TTL 63 → 62
 ▼
SERVER
```

L'utilisateur pourra avancer paquet par paquet.

```text
[◀] [Pause] [▶] [Auto]
```

---

# 12. Explain Engine

NETIV devra pouvoir expliquer les décisions réseau.

Exemple :

```text
Pourquoi R1 utilise cette interface ?
```

Réponse :

```text
Destination :
192.168.2.10

Routes disponibles :

192.168.1.0/24 → eth0
192.168.2.0/24 → eth1
0.0.0.0/0      → eth2

Longest Prefix Match :
192.168.2.0/24

Décision :
→ eth1
```

Ce moteur sera essentiel à la vocation pédagogique du projet.

---

# 13. Network Chaos Engine

NETIV pourra volontairement casser un réseau.

Commande :

```bash
netiv chaos start
```

Scénarios :

### Couche 1

* câble déconnecté ;
* interface DOWN ;
* perte de paquets ;
* latence ;
* congestion.

### Couche 2

* mauvais VLAN ;
* MAC incorrecte ;
* problème ARP ;
* boucle ;
* STP.

### Couche 3

* mauvaise IP ;
* mauvais masque ;
* mauvaise gateway ;
* route absente ;
* route incorrecte ;
* boucle de routage ;
* TTL expiration.

### Services

* DHCP indisponible ;
* DNS indisponible ;
* serveur HTTP indisponible.

---

# 14. Troubleshooting Engine

Après une panne :

```bash
netiv diagnose
```

NETIV analyse l'environnement.

Exemple :

```text
NETWORK DIAGNOSIS

[✓] Interface PC1
[✓] IP configuration
[✓] ARP
[✗] Default gateway
[✓] Routing
[✓] DNS

Root cause:

PC1 has configured gateway:
192.168.10.254

Expected:
192.168.10.1
```

---

# 15. Mode "What If?"

Permet de simuler une modification avant de l'appliquer.

```bash
netiv what-if "R1 goes down"
```

Résultat :

```text
Impact Analysis

Affected hosts: 23
Affected services: 5
Broken routes: 8
Isolated VLANs: 2
```

Autres exemples :

```bash
netiv what-if "block TCP/443"
netiv what-if "remove route 10.10.0.0/16"
netiv what-if "disconnect SW2"
```

---

# 16. Network Health Score

NETIV analysera la qualité du réseau.

```text
NETWORK HEALTH

Connectivity      95/100
Addressing        100/100
Routing           82/100
Segmentation      60/100
Availability      90/100
Security          55/100

GLOBAL SCORE      79/100
```

---

# 17. Security Analyzer

NETIV pourra détecter des problèmes de conception.

Exemples :

```text
[HIGH]
Server network is not isolated.

[HIGH]
Management interface accessible from user VLAN.

[MEDIUM]
No network segmentation.

[MEDIUM]
No redundancy.

[LOW]
DNS has no secondary server.
```

---

# 18. Cyber Range

Une partie de NETIV sera dédiée à la cybersécurité.

Architecture :

```text
                 Internet
                    │
                Firewall
                    │
          ┌─────────┴─────────┐
          │                   │
         DMZ                  LAN
          │                   │
        WEB01               PC01
        WEB02               PC02
```

L'utilisateur pourra avoir des scénarios :

### Exemple

> Une machine de la DMZ communique anormalement avec le LAN.

Objectifs :

* identifier la machine ;
* analyser le trafic ;
* comprendre le chemin ;
* bloquer le trafic ;
* corriger la segmentation.

---

# 19. Forensics Mode

Après une simulation :

```bash
netiv forensic analyze capture.nlv
```

NETIV produit une timeline :

```text
10:01:01 ARP Request
10:01:02 ARP Reply
10:01:03 TCP SYN
10:01:03 TCP SYN/ACK
10:01:04 HTTP Request
10:01:05 HTTP Response
```

Puis recherche :

* anomalies ;
* scans simulés ;
* connexions inhabituelles ;
* communications interdites ;
* changements de routes.

---

# 20. Learning Mode

NETIV devra proposer des parcours.

```text
NETWORK FUNDAMENTALS

[✓] Ethernet
[✓] MAC
[✓] ARP
[ ] IPv4
[ ] Subnetting
[ ] Routing
[ ] VLAN
[ ] DNS
[ ] DHCP
```

---

# 21. Challenge Engine

Exemple :

```text
NETWORK CHALLENGE #12

Difficulty: Medium

PC1 cannot reach SERVER.

Find and fix the problem.

Time: 20 minutes
```

Score :

```text
Diagnosis        +40
Correct fix      +40
Time             +20
Hint             -10
```

---

# 22. Génération automatique de challenges

NETIV pourra générer :

```bash
netiv challenge generate \
    --topic routing \
    --difficulty hard
```

Le système crée automatiquement :

* topologie ;
* configuration ;
* panne ;
* objectif ;
* solution ;
* système de scoring.

C'est une fonctionnalité particulièrement intéressante pour les enseignants.

---

# 23. Teacher Mode

Un enseignant pourra créer :

```text
Course
 ├── Lab
 ├── Exercise
 ├── Challenge
 └── Exam
```

Exemple :

```text
TP #04 — Routage

Objectif:
Configurer la communication entre trois réseaux.

Durée:
45 minutes

Niveau:
Intermédiaire
```

---

# 24. Student Mode

L'étudiant aura :

```text
Dashboard

XP: 2,450

Networking
████████░░ 80%

Routing
██████░░░░ 60%

Security
████░░░░░░ 40%

Challenges completed: 37
```

---

# 25. Système de progression

NETIV pourra attribuer :

* XP ;
* niveaux ;
* badges ;
* scores ;
* statistiques.

Exemple :

```text
Network Beginner
ARP Explorer
Routing Specialist
Packet Analyst
Troubleshooter
Network Defender
```

---

# 26. AI Tutor

L'IA ne doit **pas résoudre immédiatement l'exercice**.

Elle fonctionne par indices.

```text
Student:
Pourquoi le ping échoue ?

NETIV:
Indice 1 :
Vérifie la configuration IP de PC1.
```

Puis :

```text
Indice 2 :
PC1 considère-t-il SERVER comme une machine locale ?
```

Puis :

```text
Indice 3 :
Regarde le masque.
```

Finalement :

```text
Solution :
PC1 possède un masque /16 au lieu de /24.
```

L'IA doit donc être un **tuteur**, pas un générateur automatique de réponses.

---

# 27. Générateur de topologies

```bash
netiv topology generate \
    --type campus \
    --routers 5 \
    --hosts 100
```

Types :

* LAN ;
* entreprise ;
* campus ;
* datacenter ;
* ISP ;
* hybride ;
* cyber range.

---

# 28. Network Documentation Generator

NETIV pourra générer automatiquement :

```text
Network Documentation

Topology
IP addressing plan
VLAN table
Routing table
Devices
Interfaces
Services
Security recommendations
```

Export :

```text
Markdown
JSON
YAML
PDF
```

---

# 29. Import / Export

Formats internes :

```text
.netiv
.json
.yaml
```

Exemple :

```yaml
network:
  name: campus-esatic

devices:
  - name: R1
    type: router

  - name: SW1
    type: switch
```

---

# 30. CLI

NETIV sera d'abord pensé **CLI-first**.

Exemples :

```bash
netiv init
netiv topology
netiv device
netiv config
netiv run
netiv ping
netiv trace
netiv packet
netiv inspect
netiv diagnose
netiv chaos
netiv challenge
netiv capture
netiv report
```

---

# 31. Interface TUI

Une interface terminal interactive pourra être ajoutée.

```text
┌───────────────────────────────────────────────────────┐
│ NETIV — Network Côte d'Ivoire                       │
├───────────────────────────────────────────────────────┤
│                                                       │
│                 R1                                    │
│               /    \                                  │
│             SW1    SW2                                │
│            /  \    /  \                               │
│          PC1  PC2 PC3  PC4                           │
│                                                       │
├───────────────────────────────────────────────────────┤
│ Packet #42                                             │
│ ICMP Echo Request                                     │
│ PC1 → R1 → R2 → SERVER                               │
├───────────────────────────────────────────────────────┤
│ [T] Topology [P] Packets [R] Routing [D] Diagnose    │
└───────────────────────────────────────────────────────┘
```

---

# 32. Architecture technique

Je recommande une architecture en plusieurs couches.

```text
                    NETIV
                      │
              ┌───────┴───────┐
              │               │
             CLI             TUI
              │               │
              └───────┬───────┘
                      │
                Application
                      │
             ┌────────┴────────┐
             │                 │
        Simulation         Education
             │                 │
             │             Challenges
             │             Courses
             │             Scoring
             │
        Network Engine
             │
     ┌───────┼────────┐
     │       │        │
 Ethernet   IP      Transport
     │       │        │
    ARP    ICMP    TCP/UDP
             │
         Services
       DHCP / DNS
```

---

# 33. Technologies

### Langage

**Rust 2024 Edition**, avec une version stable récente. La documentation officielle indique actuellement Rust 1.97.0 et l'édition 2024. ([Documentation Rust][1])

### Asynchrone

**Tokio** pour les composants réseau nécessitant concurrence et I/O asynchrones. Tokio fournit notamment un runtime multithread et les primitives nécessaires aux applications réseau. ([Tokio][2])

### CLI

`clap`

### Sérialisation

`serde`

### Configuration

YAML / TOML

### Logs

`tracing`

### Tests

Rust unit tests + integration tests

### TUI

`ratatui`

### Documentation

`rustdoc` + Markdown.

La documentation officielle Rust recommande notamment de documenter les API publiques avec des explications et des exemples utilisables. ([Documentation Rust][3])

---

# 34. Principes Rust

NETIV doit également être un **projet d'apprentissage de Rust**.

Nous exploiterons notamment :

* ownership ;
* borrowing ;
* lifetimes ;
* `struct` ;
* `enum` ;
* traits ;
* generics ;
* iterators ;
* `Option` ;
* `Result` ;
* error handling ;
* modules ;
* concurrency ;
* async/await ;
* channels ;
* tests.

L'ownership est particulièrement important puisque Rust s'appuie dessus pour garantir la sécurité mémoire sans garbage collector. ([Documentation Rust][4])

---

# 35. Architecture du repository

Je partirais sur :

```text
netiv/
│
├── Cargo.toml
├── Cargo.lock
├── README.md
├── LICENSE
├── CONTRIBUTING.md
├── SECURITY.md
│
├── docs/
│   ├── architecture/
│   ├── protocols/
│   ├── tutorials/
│   └── labs/
│
├── examples/
│
├── scenarios/
│   ├── beginner/
│   ├── intermediate/
│   └── advanced/
│
├── tests/
│
└── crates/
    │
    ├── netiv-core/
    ├── netiv-network/
    ├── netiv-protocols/
    ├── netiv-simulator/
    ├── netiv-topology/
    ├── netiv-packets/
    ├── netiv-diagnostics/
    ├── netiv-chaos/
    ├── netiv-security/
    ├── netiv-challenges/
    ├── netiv-education/
    ├── netiv-cli/
    └── netiv-tui/
```

---

# 36. Sécurité du projet

NETIV doit respecter une séparation stricte entre :

```text
SIMULATION
     ≠
REAL NETWORK
```

Par défaut, les scénarios sont **100 % virtuels**.

Aucune fonctionnalité offensive ne doit agir sur des systèmes tiers.

Le mode réel, s'il est développé plus tard, devra nécessiter une activation explicite et être limité à l'environnement autorisé.

---

# 37. Performance

L'utilisation de Rust permettra de construire un moteur efficace et prévisible.

Les simulations devront être capables de gérer progressivement :

```text
MVP
100 devices

V2
1,000 devices

V3
10,000+ simulated devices
```

Le moteur devra éviter de créer inutilement une tâche système pour chaque équipement.

Pour les composants fortement concurrents, Tokio pourra permettre de gérer efficacement de nombreuses opérations asynchrones. ([Tokio][2])

---

# 38. Architecture modulaire

Un principe important :

> **Le moteur réseau ne doit jamais dépendre de l'interface utilisateur.**

Ainsi :

```text
             Core Engine
             /        \
           CLI        TUI
             \        /
              Web UI
```

Plus tard, on pourrait même ajouter une API :

```text
NETIV Core
     │
     ├── CLI
     ├── TUI
     ├── Web
     └── API
```

---

# 39. Roadmap officielle

## Phase 0 — Fondation

* repository ;
* Cargo workspace ;
* architecture ;
* documentation ;
* CI ;
* tests ;
* logging.

---

## Phase 1 — Network Core

Implémenter :

* devices ;
* interfaces ;
* liens ;
* MAC ;
* Ethernet ;
* IP ;
* ICMP.

Objectif :

```text
PC1 ─── PC2

ping PC1 PC2
```

---

## Phase 2 — Network Infrastructure

Ajouter :

* switch ;
* ARP ;
* routeur ;
* routing ;
* subnetting ;
* DHCP ;
* DNS.

---

## Phase 3 — Packet Visualization

Ajouter :

* packet timeline ;
* packet inspector ;
* packet journey ;
* encapsulation ;
* TTL ;
* tables.

---

## Phase 4 — Troubleshooting

Ajouter :

* chaos engine ;
* pannes ;
* diagnostic ;
* root cause ;
* hints.

---

## Phase 5 — Education

Ajouter :

* challenges ;
* scoring ;
* XP ;
* niveaux ;
* teacher mode ;
* student mode.

---

## Phase 6 — Security

Ajouter :

* firewall ;
* ACL ;
* segmentation ;
* security analyzer ;
* cyber range ;
* forensic mode.

---

## Phase 7 — Intelligence

Ajouter :

* AI tutor ;
* automatic topology generation ;
* automatic challenge generation ;
* what-if engine ;
* network health.

---

## Phase 8 — Ecosystem

Ajouter éventuellement :

* Web UI ;
* API ;
* comptes utilisateurs ;
* partage de labs ;
* marketplace/community de scénarios ;
* classement ;
* plateforme collaborative.

---

# 40. MVP officiel

Attention : **on ne commence pas avec toutes ces fonctionnalités.**

Le MVP de NETIV doit être :

```text
NETIV v0.1

✓ Create topology
✓ Create PC
✓ Create switch
✓ Create router
✓ Connect devices
✓ Configure IP
✓ Ethernet
✓ ARP
✓ IPv4
✓ ICMP
✓ Ping
✓ Packet visualization
✓ Basic routing
✓ CLI
```

Et surtout :

```text
PC1 ── SW1 ── R1 ── R2 ── SERVER

              │
              ▼
        PACKET JOURNEY
```

Si ce cœur fonctionne parfaitement, **tout le reste pourra être construit dessus**.

---

# 41. Critères de réussite

NETIV sera considéré comme réussi lorsque quelqu'un pourra :

### Construire

```text
PC1 ─ SW1 ─ R1 ─ R2 ─ SERVER
```

### Configurer

```text
IP
VLAN
Routing
DNS
DHCP
```

### Simuler

```text
ping
ARP
TCP
UDP
```

### Observer

```text
Packet
Timeline
Tables
Routes
```

### Casser

```text
Fault injection
```

### Diagnostiquer

```text
Root cause
```

### Apprendre

```text
Challenge
Hint
Score
```

### Sécuriser

```text
Firewall
ACL
Segmentation
```

---

# 42. Identité du projet

Je garderais officiellement :

## **NETIV**

### **Network Côte d'Ivoire**

**Slogan proposé :**

> **Build. Break. Understand. Secure.**

Ou une version française :

> **Construire. Comprendre. Diagnostiquer. Sécuriser.**

Logo : quelque chose de très sobre avec un **N** construit à partir de connexions réseau, plutôt qu'un logo "cyber hacker".

---

# 43. Positionnement final

NETIV ne doit donc pas chercher à battre Packet Tracer sur le terrain de la simulation Cisco.

Son terrain est différent :

| Outil         | Force principale                                              |
| ------------- | ------------------------------------------------------------- |
| Packet Tracer | Simulation pédagogique d'équipements                          |
| GNS3          | Émulation/virtualisation réseau                               |
| EVE-NG        | Labs réseau avancés                                           |
| Wireshark     | Analyse de paquets réels                                      |
| **NETIV**     | **Comprendre + simuler + casser + diagnostiquer + apprendre** |

C'est cette combinaison qui constitue notre **différenciation**.

Et surtout, le projet est suffisamment ambitieux pour devenir un **gros projet Rust sur 6–12 mois**, tout en ayant un MVP réalisable rapidement. Rust est particulièrement adapté à cette orientation système/réseau grâce à ses garanties de sécurité mémoire et ses performances, tandis que Tokio nous donnera ensuite une base solide pour les composants réseau concurrents. ([Documentation Rust][4])

**La prochaine étape logique est de transformer ce cahier des charges en architecture technique concrète : définir les structures Rust (`Device`, `Interface`, `Packet`, `Network`, `Route`, etc.), les crates du workspace et surtout le fonctionnement exact du moteur de simulation.**

[1]: https://doc.rust-lang.org/stable/book/index.html?utm_source=chatgpt.com "The Rust Programming Language - The Rust Programming Language"
[2]: https://tokio.rs/tokio/tutorial?utm_source=chatgpt.com "Tutorial | Tokio - An asynchronous Rust runtime"
[3]: https://doc.rust-lang.org/rustdoc/how-to-write-documentation.html?utm_source=chatgpt.com "How to write documentation - The rustdoc book"
[4]: https://doc.rust-lang.org/book/ch04-00-understanding-ownership.html?utm_source=chatgpt.com "Understanding Ownership - The Rust Programming Language"
