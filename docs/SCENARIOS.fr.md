# Situations de jeu (« scènes ») par sport

reco-minimap peut reconnaître les **placements sur balle arrêtée** – remises en jeu et phases arrêtées où les joueurs se tiennent à des endroits caractéristiques et bougent à peine – et, au basketball, les **tentatives de tir**. Cette page indique, pour chaque sport, quelles situations sont faciles à reconnaître, comment elles le sont et de quoi elles dépendent.

On l'active avec `--scenes` (ligne de commande) ou la case *Détecter les situations de jeu* aux étapes 4 / 5 de l'interface.

**Légende des statuts**

| | |
|---|---|
| ✅ | **Implémenté.** Règles appliquées aux positions suivies, testé sur des scènes construites. |
| 🟡 | **Implémenté, dépend du ballon.** Ne fonctionne que si le ballon est trouvé au bon moment. |
| 🔧 | **Faisable, pas encore implémenté.** Le motif est simple ; il reste à l'écrire et à le régler. |
| ❌ | **Indétectable à partir des positions.** Nécessite un tableau d'affichage, les gestes de l'arbitre ou la connaissance des règles. |

> **État réel :** toutes les scènes ✅ et 🟡 ont été vérifiées **uniquement sur des placements construits** (cas positifs et sosies typiques qui ne doivent *pas* déclencher). Elles n'ont pas été réglées sur de vrais matchs. Attendez-vous à ajuster les seuils et à des faux positifs pour les plus fragiles (signalées ci-dessous).

## Liste de priorités

| Sport | Situation | Statut |
|---|---|---|
| Basketball | Remise en jeu | ✅ |
| Basketball | Lancer franc | ✅ |
| Basketball | Changement | ✅ (numéros, ou sortie et entrée par la ligne de touche) |
| Basketball | Entre-deux | ✅ |
| Basketball | Tirs à 2 et 3 points | 🟡 (il faut voir le ballon quitter les mains du tireur ; pas s'il est rentré) |
| Football | Coup d'envoi | ✅ |
| Football | Changement | ✅ |
| Football | Penalty | ✅ |
| Football | Corner | ✅ |
| Handball | Changement | ✅ |
| Handball | Touche | ✅ |
| Handball | Jet de 7 m | ✅ |

---

## Fonctionnement

1. À chaque image traitée, le détecteur examine les joueurs suivis (positions en mètres, vitesses).
2. Un **placement** est un motif tel que « un joueur immobile sur la ligne de lancer franc, trois ou plus dans la raquette, presque tout le monde à l'arrêt ».
3. Le motif doit **tenir un instant** (1 à 2 s selon la scène) avant que l'événement soit émis, et il n'est **émis qu'une fois** par placement. Il ne peut être réémis qu'après la dissolution du placement et un délai de 6 s.
4. L'événement indique quoi, où, quelle équipe l'exécute, et une confiance.

**Conditions nécessaires**

* Un calibrage avec une erreur d'environ **0,5 m ou moins**. Les tolérances sont de 1 à 2 m.
* La plupart des joueurs suivis (environ 60 % ou plus). Un placement n'est pas reconnu si la moitié des joueurs manque.
* Pour les scènes dépendant du ballon, un ballon suivi au moment où il est lâché.

**Échecs typiques**

* *Manquée :* joueurs non détectés (côté éloigné, petite image), mauvais calibrage, placement plus court que le temps de maintien.
* *Faux positifs :* un joueur immobile sur la ligne de touche pendant le jeu ressemble à un joueur qui remet en jeu – c'est pourquoi les touches ont une confiance plus faible (0,5) sauf si le ballon est dans ses mains (0,8). Un gardien immobile ne constitue pas une scène à lui seul, raison pour laquelle les coups de pied de but ne sont pas implémentés.

## Sortie

Les scènes apparaissent à plusieurs endroits :

* **Journal :** `SITUATION lancer franc à gauche (domicile) à 12:34 (confiance 0.85)` et un récapitulatif à la fin d'un traitement. Le texte suit la langue choisie.
* **JSON** (`--json`) : dans le tableau `events` de l'image où la scène a été émise :

```json
{"type": "scene", "kind": "free_throw", "team": "home", "side": "l",
 "x": 5.8, "y": 7.5, "confidence": 0.85, "borderline": false, "t": 754.2}
```

`kind` vaut `tip_off`, `kickoff`, `free_throw`, `throw_in`, `corner`, `penalty`, `seven_meter`, `shot2` ou `shot3`. `side` vaut `l` ou `r` (l'extrémité du terrain concernée) ou `null`. `team` est l'équipe qui exécute la remise en jeu ou le tir. `borderline` est vrai pour les tirs lâchés à moins de l'erreur de position de la ligne à 3 points.

* **Incrustation en direct :** un bandeau pendant quelques secondes, dans la langue de l'interface.
* **`reco-minimap stats` :** liste chaque scène avec l'heure et la confiance.

---

## Basketball

| Situation | Statut | Comment elle est reconnue | Remarques |
|---|---|---|---|
| **Entre-deux** | ✅ | Deux adversaires dans le cercle central (≤ 1,5 m), proches l'un de l'autre, immobiles ; personne d'autre à moins de 2,4 m du centre ; les autres (≥ 6 suivis) plutôt calmes. Maintien 1 s. | Un troisième joueur dans le cercle (une mêlée) l'annule. |
| **Lancer franc** | ✅ | Un joueur immobile sur la ligne de lancer franc (≤ 1,1 m), ≥ 3 joueurs dans la zone de la raquette, au plus un joueur sur cinq en mouvement rapide. Maintien 2 s. | Indique le côté (`l`/`r`) et l'équipe du tireur. |
| **Remise en jeu** | ✅ | Un joueur immobile sur la ligne de touche ou de fond (≤ 0,8 m du bord, à l'intérieur ou juste à l'extérieur), personne d'autre à moins de 1,2 m, ≥ 3 joueurs à moins de 9 m. Maintien 1,2 s. | Confiance 0,8 si le ballon est dans ses mains ou à côté de lui, sinon 0,5. Un joueur immobile sur la ligne pendant le jeu peut la déclencher. |
| **Changement** | ✅ | Le numéro confirmé d'une place change, ou un joueur suivi en dernier près d'une ligne de touche disparaît au moins 3 s et quelqu'un apparaît près d'une ligne de touche (« possible »). Voir le [README](../README.md#jersey-numbers-and-substitutions). | Le plus fiable avec les numéros et un effectif. |
| **Tir à 2 ou 3 points** | 🟡 | Le ballon quitte les mains du porteur (état *tenu* → *en vol*) et se dirige vers un panier (finit ≥ 2 m plus près, ou à moins de 2,5 m). Classé selon la position du tireur au lâcher : ≥ 6,8 m du panier, ou dans la bande de coin, compte pour 3. Les lancers francs sont exclus. | `borderline: true` si le lâcher est à moins de 0,8 m de l'arc – l'erreur de position est de cet ordre. Manqué si le ballon n'est pas suivi au lâcher ; avec un modèle de ballon faible, c'est fréquent. |
| Réussi ou manqué | ❌ | Invisible à partir des positions. | Nécessite le tableau d'affichage. Un panier réussi est en général suivi, en quelques secondes, d'une remise en jeu de l'*autre* équipe depuis la ligne de fond – un indice, pas une preuve. |
| Temps mort | 🔧 | Tout le monde immobile et regroupé côté banc pendant > 20 s. | |
| Contre-attaque | 🔧 | De nombreux joueurs des deux équipes franchissant la ligne médiane dans le même sens à grande vitesse. | |
| Faute, violations, ballon tenu | ❌ | Connaissance des règles et gestes de l'arbitre. | |

## Football

| Situation | Statut | Comment elle est reconnue | Remarques |
|---|---|---|---|
| **Coup d'envoi** | ✅ | Les deux équipes dans leur moitié (≥ 85 % chacune), toutes immobiles, au plus deux joueurs d'une même équipe dans le rond central (9,15 m). Maintien 1,5 s. | L'équipe présente dans le rond l'exécute. Émis au début de chaque mi-temps et après un but. |
| **Penalty** | ✅ | Un joueur immobile au point de penalty (≤ 1,8 m), personne d'autre à moins de 8,5 m, au plus un autre joueur (le gardien) dans la surface de réparation. Maintien 2 s. | Indique le côté et l'équipe du tireur. |
| **Corner** | ✅ | Un joueur immobile près d'un poteau de corner (≤ 2,4 m), ≥ 5 joueurs dans la surface de réparation de ce côté, et ces joueurs se déplaçant lentement. Maintien 1,5 s. | |
| **Touche** | ✅ | Comme au basketball, sur une ligne de touche, hors de la zone de corner (le corner est laissé au détecteur de corner). | Mêmes règles de confiance que ci-dessus. |
| **Changement** | ✅ | Comme ci-dessus. Un remplaçant entre en général à hauteur de la ligne médiane, près de la ligne de touche. | |
| Coup de pied de but | 🔧 | Tireur dans la surface de but, adversaires hors de la surface de réparation. | Non implémenté : un gardien immobile a le même aspect pendant le jeu. Nécessite le ballon. |
| Coup franc (mur) | 🔧 | ≥ 3 défenseurs alignés à environ 10 m du ballon, immobiles. | |
| But | 🔧 | Un coup d'envoi de l'*autre* équipe suit dans la minute ; un tableau d'affichage est plus sûr. | Impossible à distinguer du début d'une mi-temps sans le chronomètre. |
| Hors-jeu, faute, cartons | ❌ | | |

## Futsal

| Situation | Statut | Comment elle est reconnue | Remarques |
|---|---|---|---|
| **Coup d'envoi** | ✅ | Comme au football, rond central de 3 m. | |
| **Rentrée de touche** | ✅ | Joueur immobile sur une ligne de touche (détecteur de touche). | |
| **Corner** | ✅ | Comme au football, avec ≥ 3 joueurs dans la surface. | |
| **Penalty (6 m)** | ✅ | Tireur au point des 6 m (≤ 1,5 m), personne à moins de 4,5 m, au plus un autre joueur dans la surface. | |
| **Changement** | ✅ | Comme ci-dessus. | Les changements volants sont fréquents ; attendez-vous à de nombreux changements *possibles*. |
| Second penalty (10 m) | 🔧 | | |
| Temps mort | 🔧 | | |

## Handball

| Situation | Statut | Comment elle est reconnue | Remarques |
|---|---|---|---|
| **Engagement** | ✅ | Les deux équipes dans leur moitié, immobiles, le tireur au centre. | Noté « kick-off » dans les données (`kickoff`) et « Engagement » dans l'interface. |
| **Jet de 7 m** | ✅ | Tireur immobile à la marque des 7 m (≤ 1,2 m), personne d'autre à moins de 1,9 m, au plus un autre joueur dans la zone des 6 m. Maintien 2 s. | |
| **Touche** | ✅ | Joueur immobile sur une ligne de touche. | Les jets de coin, exécutés sur la ligne de touche près du coin, sont inclus. |
| **Changement** | ✅ | Comme ci-dessus. | Les changements volants sont fréquents ; attendez-vous à de nombreux changements *possibles*. |
| Jet franc | 🔧 | Un mur défensif de ≥ 3 joueurs à environ 3 m du tireur. | |
| Renvoi | 🔧 | Gardien immobile dans la zone des 6 m. | Nécessite le ballon (même problème que le coup de pied de but au football). |
| Exclusion de 2 minutes | 🔧 | Une équipe joue avec un joueur de moins pendant deux minutes : compter les places suivies de cette équipe. | |
| Temps mort | 🔧 | | |

## Hockey (sur gazon)

| Situation | Statut | Comment elle est reconnue | Remarques |
|---|---|---|---|
| **Passe d'engagement** | ✅ | Le motif du coup d'envoi avec un rayon central de 5 m. | |
| **Stroke** | ✅ | Tireur au point des 6,4 m (≤ 1,5 m), personne à moins de 4,5 m, au plus un autre joueur dans le cercle. | |
| **Changement** | ✅ | Comme ci-dessus. | Les changements tournants sont fréquents. |
| Petit corner | 🔧 | Injecteur sur la ligne de but, attaquants hors du cercle, défenseurs derrière la ligne. | |
| Grand corner, coup franc | 🔧 | | |

## Rugby

| Situation | Statut | Comment elle est reconnue | Remarques |
|---|---|---|---|
| **Coup d'envoi** | ✅ | Équipes dans leur moitié, immobiles, au plus deux joueurs d'une même équipe à moins de 10 m du centre. | |
| Mêlée | 🔧 | 14 à 16 joueurs liés dans un cercle d'environ 4 m, presque immobiles. | |
| Touche | 🔧 | Deux lignes parallèles de joueurs près de la ligne de touche. | |
| Pénalité ou transformation | 🔧 | Un botteur seul, les autres en retrait. | |
| Changement | 🔧 | La logique de changement doit être réglée pour 15 places par équipe. | |
| Essai | ❌ | Nécessite le ballon aplati et le tableau d'affichage. | |

## Crosse

| Situation | Statut | Comment elle est reconnue | Remarques |
|---|---|---|---|
| **Mise au jeu** | ✅ | Deux adversaires au centre (≤ 1,2 m), les autres autour, immobiles. | |
| Changement | 🔧 | Changements à la volée par la zone de changement. | |
| But, supériorité numérique | ❌ | | |

## Football américain

| Situation | Statut | Comment elle est reconnue | Remarques |
|---|---|---|---|
| Ligne de mêlée / formation au snap | 🔧 | Deux lignes de joueurs face à face dans une bande d'environ 10 yards, presque immobiles pendant ≥ 5 s. Un motif net ; une bonne prochaine étape. | |
| Coup d'envoi | 🔧 | Une ligne de botteurs en travers du terrain, receveurs en profondeur. | |
| Formations de dégagement, de placement | 🔧 | | |
| Temps mort | 🔧 | | |
| Touchdown, premier essai | ❌ | | |

## Volleyball

| Situation | Statut | Comment elle est reconnue | Remarques |
|---|---|---|---|
| Service | 🔧 | Un joueur seul derrière la ligne de fond, immobile. | |
| Changement | 🔧 | Joueurs dans la zone de changement. | |
| Rotation | 🔧 | Changement de position dans le sens horaire après un changement de service. | |
| Point | ❌ | | |

## Autres sports (rectangle libre)

| Situation | Statut |
|---|---|
| Touche / remise en jeu sur la ligne de touche | ✅ |
| Changement | ✅ |
| Tout le reste | ❌ (pas de géométrie propre au sport) |

---

## Pour les développeurs : ajouter une scène

Le détecteur se trouve dans [`src/scenes.rs`](../src/scenes.rs). Une scène est une fonction qui examine les joueurs d'une image et propose un candidat (`kind`, `key`, `team`, `pos`, `side`, `confidence`). Le code de temporisation commun gère le temps de maintien, l'émission unique, le délai et le réarmement. Étapes :

1. Ajouter une variante de `SceneKind`, sa chaîne `key()` et son temps `hold()`.
2. Écrire la fonction du motif (voir `free_throws` ou `corners` comme modèles) et l'appeler pour les bons sports dans `patterns()`.
3. Ajouter un test dans `tests/scenes.rs` : un placement positif et le sosie le plus proche qui ne doit **pas** déclencher.
4. Ajouter les noms dans `scripts/gen_i18n.py` (`scene.<kind>`) et dans `scene_name` de `src/bin/reco-minimap.rs`, dans les quatre langues, puis lancer le script.

Briques utiles : `profile.landmark("spot_l")`, `profile.zone_rect('l', 1)` (surface de réparation / raquette), `profile.field.edge_dist(pos)`, `AgentOut.vel`, et l'état du ballon dans l'image.
