# Performance

## Modèle de coût

| Opération | Coût | Remarque |
|---|---|---|
| Un événement du moteur | O(log n) | File de priorité (date, numéro d'ordre). Le moteur est mono-thread et déterministe. |
| Routage d'un paquet | O(routes de l'équipement) | Toutes les routes candidates sont conservées dans la trace : c'est ce qu'affiche `explain`. |
| Ajout à la trace | O(1) amorti | Index `transaction → événements` tenu à jour au fil de l'eau. |
| `trace <id>`, parcours web | O(taille de la transaction) | Grâce à l'index, indépendant de la longueur de la session. |
| Copie neuve (`fresh_clone`) | O(configuration) | La trace et la file d'événements ne sont **pas** copiées. |
| Une sonde (diagnostic, what-if…) | 1 copie neuve + 1 mini-simulation | La simulation de l'utilisateur n'est jamais modifiée. |
| Matrice d'accessibilité | O(flux) sondes, **en parallèle** | Plafonnée à 4 000 flux (`MAX_FLOWS`), échantillonnés à pas régulier au-delà. |

Les sondes sont indépendantes : chacune travaille sur sa propre copie. Elles sont donc
réparties sur les cœurs (`std::thread::scope`, aucune dépendance), et les résultats sont
rangés dans l'ordre d'entrée : **même résultat avec 1 ou 16 threads**, ce que vérifient les tests.

## Mesurer : `netiv bench`

```bash
cargo build --release
./target/release/netiv bench                          # datacenter, 20 routeurs, 120 postes, 2 000 pings
./target/release/netiv bench --routers 40 --hosts 300 --pings 5000
```

Le réseau est généré avec une graine fixe : deux exécutions mesurent exactement le même travail.
Les analyses sont mesurées sur une simulation **chargée de toute sa trace**, le cas réel d'une
longue session.

Exemple (8 cœurs, build release) :

```text
NETIV BENCH — datacenter, 164 équipements, 180 câbles, 8 cœur(s)

Chargement de la topologie   1.2 ms
Moteur                       2000 pings : 28135 événements traités (88406 entrées de trace) en 34.8 ms → 809322 év./s
Trace indexée                1000 transactions (44405 événements) en 0.1 ms
Copie neuve ×100             7.3 ms (trace de 88406 événements ignorée)
Matrice d'accessibilité      3450 flux : 376.4 ms séquentiel, 93.9 ms parallèle (×4.0)
Analyse de sécurité          27.1 ms
Score de santé               106.2 ms
What-if « R1 goes down »     215.2 ms
```

## Avant / après l'optimisation (v0.3)

Mêmes scripts, même réseau généré (`datacenter`, 20 routeurs, 120 postes), même machine
(8 cœurs), temps total du processus :

| Scénario | Avant | Après | Cause corrigée |
|---|---|---|---|
| `ping … 20000` | 85 s | **0,45 s** | Le rapport de ping cherchait chaque résultat dans toute la trace (quadratique). |
| `ping … 2000` | 0,42 s | **0,05 s** | Idem. |
| `ping … 2000` puis `health` | 15,4 s | **0,16 s** | Chaque sonde recopiait toute la trace de la session. |
| `health` (lab neuf) | 0,55 s | **0,11 s** | Sondes en parallèle ; pings de l'analyse de sécurité préchauffés et réutilisés. |
| `what-if "R1 goes down"` | 0,81 s | **0,21 s** | Sondes en parallèle. |
| `security analyze` | 0,19 s | **0,04 s** | Pings en parallèle ; le chemin n'est plus mesuré une seconde fois. |

Piste étudiée puis écartée : mettre en `Box` les grosses variantes d'événement de trace. La taille
d'un événement passe de 144 à 96 octets, mais la mémoire maximale mesurée augmente (allocations
supplémentaires). Le changement n'a pas été gardé.

## Limites connues

- **Mémoire de la trace** : environ 300 octets par entrée ; 20 000 pings à travers 6 sauts font
  près de 900 000 entrées, soit environ 260 Mo. La trace n'est jamais tronquée, puisque c'est elle
  qu'on explore. Pour repartir de zéro, il suffit de recharger le réseau ; côté Rust,
  `Simulation::clear_trace` la vide.
- **Échantillonnage** : au-delà de 4 000 flux, la matrice d'accessibilité (santé, what-if) mesure
  un échantillon régulier et déterministe. Un réseau de 300 postes a environ 90 000 couples.
- **Moteur mono-thread** : c'est voulu, pour le déterminisme. Seules les sondes indépendantes
  sont parallélisées.
