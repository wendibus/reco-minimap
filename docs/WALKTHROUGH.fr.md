# Guide pas à pas : de deux vidéos de caméra à la minimap

Ce guide vous mène de « j'ai une vidéo panoramique » à « j'ai une vidéo de minimap ». Il suit les sept étapes de l'interface. La première fois, comptez environ 30 minutes ; pour les matchs suivants, quelques minutes de clics plus le temps de calcul.

<p><img src="media/demo-pip.webp" alt="Résultat" width="58%"> <img src="media/demo-minimap.webp" alt="Résultat" width="40%"></p>

**Voici le résultat :** les huit premières secondes d'une vidéo de test animée (chats rouges contre suricates bleus). À gauche la vidéo de contrôle avec cadres et minimap incrustée, à droite la minimap seule. En vidéo : [vidéo de contrôle (MP4)](media/demo-pip.mp4) · [minimap (MP4)](media/demo-minimap.mp4). Toutes les images de ce guide viennent de ce projet de test.

**Ce qu'il vous faut**

* Le programme installé, voir [Installation](#installation) ci-dessous.
* Une **vidéo panoramique de tout le terrain** – l'image assemblée par Reco, **pas** le cadrage piloté par l'IA. Si vous n'avez qu'une vidéo gauche et une vidéo droite, assemblez-les d'abord avec Reco. Un export en 1920×1080 suffit pour la minimap ; pour les numéros de maillot il en faut un plus grand (voir l'étape 4).
* Un **modèle de détection** – pas obligatoire : sans modèle à vous, le programme utilise le modèle général de **Reco camera** (téléchargé une fois à l'étape 1, 10,4 Mo). Un modèle entraîné dans **Reco Trainer** pour votre sport est plus précis, surtout pour le ballon (dossier contenant `runtime.json`, `model-card.json` et un fichier `.onnx`).
* Facultatif : les numéros des deux effectifs, si vous voulez les afficher sur les points.

Lancer l'interface :

```bash
reco-minimap ui
```

Le navigateur ouvre `http://127.0.0.1:8788/`. En haut à droite, choisissez la langue (Deutsch · English · Français · Español) ; les messages du programme dans le journal suivent ce choix. En haut se trouve aussi le **dossier du projet** – tout ce qui concerne une salle ou une saison y est rangé. Par défaut `~/reco-minimap-projekte/standard` ; *Parcourir …* permet d'en choisir un autre.

> Conseil : utilisez un dossier de projet par salle et par position de caméra. Le calibrage dépend de l'emplacement de la caméra.

## Installation

| | |
|---|---|
| Rust | chaîne d'outils stable (compilé et testé avec la **1.91**) |
| ffmpeg + ffprobe | dans votre `PATH` (`brew install ffmpeg`) |
| ONNX Runtime | bibliothèque partagée **1.24 ou plus récente** |
| Un modèle de détection | par défaut : le modèle de Reco camera (l'interface le télécharge à l'étape 1) ; mieux : un export RF-DETR de Reco Trainer pour votre sport |

**macOS**

```bash
brew install ffmpeg
python3 -m venv ~/ortvenv
~/ortvenv/bin/pip install onnxruntime
find ~/ortvenv -name "libonnxruntime*.dylib"
export ORT_DYLIB_PATH="<le chemin affiché>"
cargo install --path . --features tools,coreml
```

La bibliothèque du paquet Python contient CoreML ; celle de Homebrew non. Ajoutez la ligne `export ORT_DYLIB_PATH=…` à `~/.zshrc` pour qu'elle reste valable dans les nouvelles fenêtres de terminal.

**Linux**

```bash
sudo apt install ffmpeg
python3 -m venv ~/ortvenv && ~/ortvenv/bin/pip install onnxruntime
export ORT_DYLIB_PATH=$(find ~/ortvenv -name "libonnxruntime.so*" | head -1)
cargo install --path . --features tools
```

Testé sur macOS (Apple Silicon) ; la suite de tests tourne sous Linux ; Windows n'est pas testé.

## Comment l'outil est pensé

Le travail se fait en deux temps : **régler une fois**, puis **traiter chaque match**.

**Une fois par salle et par position de caméra** (étapes 1 à 3)

1. Choisir le sport et fixer le modèle : le modèle par défaut de Reco camera ou un modèle importé de Reco Trainer.
2. Choisir une vidéo panoramique de cette salle.
3. Calibrer : cliquer les repères du terrain, mesurer la taille des joueurs, fixer les couleurs de maillot.

Le résultat se trouve dans le dossier du projet (`court.json`, `scales.json`, `teams.json`) et reste valable tant que la caméra est au même endroit et que le panorama est exporté de la même façon. Si l'un des deux change, refaites l'étape 3.

**Pour chaque match** (étapes 2, 3c, 4 et 6)

1. Choisir la nouvelle vidéo (étape 2).
2. Détecter à nouveau les couleurs de maillot, ou les régler à la main, si d'autres équipes jouent (étape 3c).
3. Faire un **court essai** de 20 à 60 secondes avec la vidéo de contrôle et le regarder (étape 4).
4. Si tout va bien, traiter le match entier par segments.
5. Regarder les chiffres de l'étape 6 si quelque chose vous paraît étrange.

**Ce que le programme décide et ce que vous fournissez**

* Il trouve les joueurs, les arbitres et le ballon, les suit et garde un nombre fixe de places par équipe. Un joueur masqué reste à sa position estimée au lieu de disparaître.
* Vous indiquez où se trouve le terrain, quelles couleurs portent les équipes et quelle plage de temps traiter. Ces trois indications déterminent la qualité ; tout le reste a des valeurs par défaut utilisables.
* La vidéo de contrôle (*Panorama avec cadres et minimap*) est votre outil de vérification. Regardez-la pour le premier segment avant de lancer un match entier.

**Disposition.** À gauche se trouvent le dossier du projet, le sport et les sept étapes ; à droite l'étape choisie. En haut, **💬 Astuces** active ou désactive les explications qui apparaissent quand la souris reste sur un bouton ; à côté, vous choisissez la langue.

**Les opérations longues** (traitement, taille des joueurs, couleurs de maillot, test du modèle, clips) ont leur propre fenêtre avec une barre de progression, le temps écoulé et le temps restant estimé. *–* la réduit en une pastille en bas à droite ; le travail continue. *Annuler* demande ce qu'il faut faire du résultat : **Arrêter, garder le résultat** termine proprement les fichiers – ils contiennent tout jusqu'à l'arrêt et se lisent normalement –, **Annuler et supprimer** efface les fichiers de ce traitement.

**En bas de la page**, le journal peut être déplié. Il affiche chaque commande lancée par l'interface et sa sortie. Tout ce que fait l'interface fonctionne aussi en ligne de commande ; quelques réglages fins n'existent que là (`reco-minimap --help`).

**Les nouvelles versions** sont signalées en haut par un bandeau bleu, voir [Mise à jour](#mise-à-jour).

---

## Étape 1 · Sport et modèle

1. **Choisir le sport.** La liste correspond à Reco Trainer. La ligne grise en dessous indique la taille du terrain, le nombre de joueurs par équipe et les places d'arbitre. Pour un sport absent de la liste, choisissez l'entrée « autre sport (terrain rectangulaire) » et saisissez longueur, largeur, nombre de joueurs et d'arbitres.
2. **Choisir le modèle.** Sans choix de votre part, le programme utilise le **modèle de Reco camera** (*Reco camera (YOLO26n)*, marqué *Par défaut* et *tous les sports*). C'est le même modèle général avec lequel Reco camera pilote sa caméra IA : il trouve personnes et ballons dans tous les sports sans entraînement, mais ne connaît pas les arbitres – le programme les reconnaît à la couleur du maillot (étape 3c).
   * La première fois, il affiche *Télécharger le modèle* : le modèle est téléchargé une fois pour tous les projets (10,4 Mo) et sa somme de contrôle vérifiée.
   * En dessous, **Modèles Reco Trainer pour** votre sport indique combien vous en avez importé et ce que le hub de modèles de Reco Trainer propose pour ce sport. Si un modèle adapté est importé, un clic le sélectionne (*Utiliser …*). Un modèle entraîné pour le sport trouve en général bien mieux le ballon et les arbitres.
   * Le chargement direct depuis le hub viendra plus tard ; en attendant, chargez un modèle dans Reco Trainer, exportez-le et importez-le ici.
3. **Importer le modèle.** Utilisez le bouton d'import de dossier et choisissez le dossier issu de Reco Trainer (ou le bouton d'import d'un fichier `.onnx` seul). Vous pouvez aussi saisir un chemin et l'importer.
   * Le programme vérifie la somme de contrôle des poids. Une somme correcte est l'objectif ; un fichier endommagé est refusé.
   * Il affiche les classes (pour les modèles Reco Trainer p. ex. `ball, player, referee, ignore`, pour le modèle par défaut `person, sports ball`), la taille d'entrée et le sport pour lequel le modèle a été entraîné. Si le modèle indique un sport, celui-ci est sélectionné pour vous. S'il diffère de votre choix, un avertissement rouge apparaît.
4. **Vérifier le modèle** (nécessite une vidéo de l'étape 2, vous pouvez donc y revenir plus tard) : le bouton de vérification fait tourner le modèle sur quelques images et écrit un rapport dans le journal. À surveiller :
   * *Joueurs par image (médiane)* proche du nombre de joueurs sur le terrain (10 au basketball). Nettement moins signifie que le modèle manque des joueurs à cette taille – continuez quand même, l'étape 3 améliore cela.
   * Le temps réel par image. Sur un M4 Pro avec CoreML, quelques centaines de millisecondes.
   * Il indique si les hypothèses sur la sortie du modèle sont vérifiées (« les boîtes sont dans 0..1 », « la plupart des détections sont sur 'player' »).

![Étape 1](img/fr/step1.webp)
*Étape 1 : sport choisi, modèle importé, somme de contrôle correcte.*

## Étape 2 · Vidéo

Choisissez la **vidéo panoramique** (*Parcourir …* ou collez le chemin). Le programme lit la taille, la cadence et la durée et les affiche en vert, p. ex. `1920×1080, 25 images/s, 20:00 min`. Un message rouge signifie que le fichier est illisible – vérifiez que `ffmpeg` est installé.

![Étape 2](img/fr/step2.webp)
*Étape 2 : la vidéo est lue – taille, cadence et durée s'affichent en vert.*

## Étape 3 · Calibrage

Le calibrage apprend au programme où se trouve le terrain dans l'image. Trois choses se passent ici, dans cet ordre.

![L'étape 3 en un coup d'œil](img/fr/step3.webp)
*L'étape 3 en un coup d'œil : calibrage (15 points, erreur 0,30 m), tailles de joueurs mesurées et couleurs des maillots.*

### 3a · Calibrer le terrain

1. Réglez *Instant dans la vidéo* sur un moment où **tout le terrain est visible** et non masqué par des personnes (p. ex. un temps mort ou l'échauffement).
2. Cliquez *Lancer le calibrage*. Une page s'ouvre dans le navigateur avec cette image.
3. **Cliquez les points dans l'ordre de la liste à droite** (coins, ligne médiane, centre, coins des zones …). La liste est dans votre langue.
   * « loin » et « proche » désignent les deux lignes de touche (loin = la plus éloignée de la caméra), « gauche/droite » comme dans l'image.
   * Au-dessus de la liste, un **petit terrain vu de dessus** montre où se trouve le point demandé : il **clignote en orange**. Les points déjà placés sont verts, les autres gris. Dans ce dessin, la caméra est en bas et « loin » en haut.
   * Vous n'êtes pas obligé de tout cliquer. Les points invisibles se passent avec la **barre d'espace**.
   * Molette = zoom, glisser = déplacer, **Retour arrière** = retirer le dernier point, **F** = image entière. La petite fenêtre en haut à droite agrandit la zone sous le pointeur.
4. À partir du 4ᵉ point, un **terrain vert** apparaît sur l'image et la page indique l'**erreur en mètres**. Ajoutez des points jusqu'à ce que les lignes vertes se superposent aux vraies lignes. Objectif : **8 points ou plus, répartis sur tout le terrain**, erreur **inférieure à 0,5 m** (vert). Des points à des endroits différents comptent plus qu'un clic très précis.
5. Cliquez *Enregistrer*. L'interface affiche ensuite p. ex. `15 points, erreur 0.21 m`.

![La page de calibrage](img/fr/calibration.webp)
*La page de calibrage : neuf points placés, les lignes vertes tombent sur le terrain. Dans le petit schéma à droite, le point 10 – le suivant – clignote ; la loupe en haut montre l'endroit sous le pointeur.*

> Une erreur de 0,00 m après exactement quatre points ne veut rien dire – quatre points s'ajustent toujours parfaitement. Jugez d'après les lignes vertes et ajoutez des points.

Si votre salle n'a pas les dimensions standard, ne cliquez que les quatre coins (et le centre) ; les autres points supposent un terrain standard.

### 3b · Mesurer la taille des joueurs

Cliquez *Mesurer*. Le programme examine une douzaine d'images et apprend quelle taille a un joueur à chaque ligne de l'image. Ensuite, chaque bande de l'image est analysée avec la taille de tuile adaptée (`--tile auto`) : la détection s'améliore et devient environ deux fois plus rapide. Cela nécessite le calibrage de 3a. Une fois par position de caméra.

### 3c · Couleurs de maillot

1. Cliquez *Détecter les couleurs*. Le programme relève les couleurs de maillot des joueurs sur le terrain et les répartit en deux équipes. Le journal montre les groupes de couleurs trouvés.
2. Réglez les couleurs :
   * **Domicile / Extérieur (détection)** – les couleurs telles que la caméra les voit. Elles sont renseignées automatiquement ; *Inverser* échange domicile et extérieur si besoin. En cas d'échec (« équipes indiscernables »), choisissez les deux couleurs à la main.
   * **Maillots de gardien** (football, futsal, handball, hockey, crosse) – si les gardiens portent une autre couleur que leur équipe, cocher *Maillots de gardien*. *Détecter les couleurs* propose les couleurs des gardiens depuis les surfaces ; si l'attribution à l'équipe est incertaine, le journal le signale – utiliser alors *Échanger les couleurs des gardiens*. Sans cela, un gardien en maillot différent est pris pour un arbitre ou ignoré. Basketball et les sports sans maillot de gardien n'affichent pas l'option. Sur la minimap, les gardiens sont dessinés rayés.
   * **Domicile / Extérieur (affichage)** – les couleurs dans lesquelles les points sont dessinés. Prenez ce qui rend bien, p. ex. du blanc et un bleu clair. Une couleur très sombre se voit mal sur la minimap sombre.
   * **Couleur des arbitres** – la couleur du maillot des arbitres (souvent noir ou jaune). Le programme trouve ainsi les arbitres même si le modèle ne les reconnaît pas.
3. **Silhouettes et maillots** (à ne modifier qu'en cas de besoin) :
   * **Zone du maillot, haut / bas** – quelle partie de la boîte du joueur montre le maillot, en proportion de la hauteur depuis le haut. La valeur par défaut, 0,18 à 0,5, convient pour des personnes. Adaptez-la pour des silhouettes aux proportions différentes (mascottes, animations), puis relancez *Détecter les couleurs*.
   * **Rapprocher l'échantillon des couleurs de maillot** – utile pour des couleurs vives et nettement différentes (rouge contre bleu) et lorsque des zones du terrain sont peintes dans une couleur d'équipe. À laisser désactivé pour clair contre foncé : dans ce cas, davantage de joueurs sont mal attribués.
   * **Match sans arbitres** – personne n'est alors pris pour un arbitre à cause d'une couleur de maillot inhabituelle.

![Couleurs des maillots et silhouettes](img/fr/step3-colours.webp)
*Couleurs des maillots et silhouettes : pour les personnages animés de la vidéo de test, la zone du maillot est plus basse (0,3 à 0,65) et l'échantillon de couleur s'appuie sur le rouge et le bleu.*

## Étape 4 · Traitement

1. **Segments.** Chaque ligne est un traitement : *Nom*, *Début* et *Fin* sous la forme `min:s` (ou `h:min:s`, ou des secondes). Une fin vide signifie jusqu'à la fin de la vidéo ; vous pouvez ainsi n'analyser qu'une partie d'un long enregistrement. Découper un match en quart-temps est plus sûr – une interruption coûte un quart-temps, pas le match entier. *Ajouter un segment* ajoute une ligne.
2. **Sorties** (cochez) :
   * *Vidéo de la minimap* – la minimap sur fond sombre.
   * *Overlay avec transparence (.mov)* – la minimap sur fond transparent, à incruster dans un logiciel de montage ou OBS. *Fond sombre de l'overlay* (0–1) ajoute une surface sombre semi-transparente pour qu'elle reste lisible sur une image claire ; 0,6 est un bon début.
   * *Panorama avec cadres et minimap (contrôle)* – votre panorama avec les boîtes de détection et la minimap dans un coin. Utilisez-le pour **vérifier la qualité** sur le premier segment.
   * *Données (.jsonl)* – une entrée par image avec toutes les positions.
3. **Détecter toutes les N images.** 5 est un bon début : la détection tourne une image sur 5, la vidéo de la minimap est interpolée à la cadence complète et reste fluide. Plus grand, c'est plus rapide ; plus petit, plus précis.
4. **Opacité des joueurs non détectés.** 1 laisse un joueur masqué entièrement visible à sa position estimée ; des valeurs plus faibles l'estompent.
5. **Calcul.** *automatique* choisit CoreML sur un Mac. Laissez *CoreML* sur `all`.
6. Cliquez **Démarrer**. Une fenêtre montre la progression (avec plusieurs sections, aussi celle en cours) et, à la fin, les fichiers écrits. *Annuler* demande s'il faut garder ou supprimer ce qui est déjà traité.

**Où sont mes fichiers ?** Dans `<dossier du projet>/out/`, nommés d'après le segment (`segment1.mp4`, `segment1.mov`, `segment1_pip.mp4`, `segment1.jsonl`). La liste sous les réglages montre les fichiers les plus récents avec *Afficher dans le dossier*.

**Commencez par un court essai.** Prenez un segment de 20 à 60 secondes avec la case *Panorama avec cadres et minimap* cochée. Ouvrez `…_pip.mp4` et vérifiez :

* Les points sont-ils sur les bonnes personnes et aux bons endroits du terrain ? Sinon, le calibrage a besoin de points plus nombreux ou mieux répartis (étape 3a).
* Y a-t-il des points fantômes ou qui sautent ? Quelques sauts sont normaux ; beaucoup indiquent un mauvais calibrage ou des couleurs trop proches.
* Les arbitres sont-ils marqués ? Sinon, vérifiez la couleur des arbitres (étape 3c).

Traitez ensuite le match entier.

![Étape 4](img/fr/step4.webp)
*Étape 4 : un segment (0:00 à 0:08), les quatre sorties cochées. Le journal en bas montre le résumé du traitement.*

![Les fichiers terminés sous le bouton Démarrer](img/fr/step4-results.webp)
*Les fichiers terminés sous le bouton Démarrer : le nom ouvre le fichier, *Afficher dans le dossier* le sélectionne dans le Finder, le chemin ouvre le dossier des résultats.*

### Numéros de maillot (facultatif)

À l'étape 4 se trouve la section *Lire et afficher les numéros de maillot*.

1. Cliquez une fois *Télécharger le modèle* (environ 11 Mo). L'état passe à « Modèle de numéros installé ».
2. Saisissez les **numéros des effectifs** des deux équipes, séparés par des virgules (`4,7,10,12,23`). La lecture ne choisit alors que parmi ces numéros – bien plus fiable. Au football, handball, hockey etc., marquez les gardiens par `g` après le numéro (`1g,4,7,12g`) : une fois la couleur du maillot de gardien réglée (étape 3c), le numéro d'un gardien n'est cherché que parmi les gardiens et celui des autres que parmi les joueurs de champ.
3. Cochez *Lire et afficher les numéros de maillot* et lancez un segment.
4. **La taille décide.** Un numéro est lisible à partir d'environ 10 px de hauteur de chiffre, soit des joueurs d'environ **80 px de haut**. À la fin du traitement, le journal indique la taille des joueurs dans votre vidéo et si cela suffit. Sinon, exportez avec Reco une version plus grande du même panorama (même ligne de temps, même cadence) et indiquez-la dans *Panorama en plus haute résolution pour les numéros*. La détection continue sur la petite vidéo.
5. Les numéros apparaissent sur les points une fois confirmés. Les joueurs dont le dos n'est jamais visible restent sans numéro.

**Les changements** apparaissent dans le journal (`CHANGEMENT domicile à 12:34 : #7 -> #12` pour un changement confirmé, « changement possible » lorsqu'un joueur a quitté le terrain par une ligne de touche et que quelqu'un est entré, sans que les numéros soient connus), dans `events` du fichier de données et à l'étape 6.

### Situations de jeu (facultatif)

Cochez *Détecter les situations de jeu* (à l'étape 4, présent aussi à l'étape 5). Le programme reconnaît alors les **placements lors des arrêts de jeu** et, au basketball, les **tentatives de tir** :

* Basketball : entre-deux, lancer franc, remise en jeu, tir à 2 et à 3 points, changement.
* Football : coup d'envoi, penalty, corner, touche, changement.
* Handball : engagement, jet de 7 m, touche, changement.
* Futsal, hockey, rugby, crosse : voir le [catalogue par sport](SCENARIOS.fr.md).

À quoi s'attendre :

* Une situation est signalée lorsque le placement a tenu un instant – p. ex. un joueur sur la ligne de lancer franc, trois dans la raquette, tous les autres immobiles. Le journal affiche alors p. ex. `SITUATION lancer franc à gauche (domicile) à 12:34 (confiance 0.85)`.
* **Vérifiez d'abord sur un court segment** en comparant avec la vidéo. Les seuils sont de premières estimations ; une touche peut être signalée lorsqu'un joueur se tient simplement sur la ligne, et une situation est manquée si beaucoup de joueurs ne sont pas détectés.
* **Les tirs à 2 et 3 points** nécessitent que le ballon soit trouvé au moment du lâcher. Ils indiquent le type de tentative, pas si elle a réussi. La mention « près de la ligne » signifie que l'erreur de position est du même ordre que la distance à la ligne à 3 points.
* Les scènes figurent dans le fichier de données comme événements avec `"type": "scene"` et à l'étape 6.

## Étape 5 · Direct (facultatif)

Le mode direct fournit une page d'incrustation transparente pour OBS pendant que la vidéo défile. Il est destiné aux diffusions ; pour le montage, utilisez l'overlay `.mov` de l'étape 4.

1. *Source* : un fichier ou une adresse de flux (`srt://…`, `rtmp://…`, `udp://…`). Pour un premier test, choisissez un fichier vidéo et cochez *Lire le fichier en temps réel*.
2. Saisissez *Taille* (p. ex. `1920x1080`) et *Images/s*. Laissez le port (8765), sauf s'il est occupé.
3. Cliquez *Démarrer le direct*. La page affiche l'adresse de l'incrustation, p. ex. `http://127.0.0.1:8765/`.
4. **Dans OBS :** ajoutez une source *Navigateur*, collez l'adresse, largeur 1400 et hauteur 800. Le fond reste transparent.
5. Ouvrez l'adresse avec `?floor=1&debug=1` dans un navigateur ordinaire pour tester sans OBS (sol sombre, ligne d'état).
6. *Arrêter* y met fin.

Le mode direct nécessite le même panorama complet que celui du calibrage, pas le cadrage piloté par l'IA.

![Étape 5 pendant un direct](img/fr/step5.webp)
*Étape 5 pendant un direct : *Adresse de l'overlay* donne l'adresse pour OBS et un lien de test.*

![Overlay en direct](img/overlay.webp)
*L'overlay en direct avec `?floor=1` (fond sombre pour tester) ; dans OBS le fond est transparent. L'anneau orange indique qui a le ballon.*

## Étape 6 · Évaluation

Les étapes 6 et 7 travaillent toutes deux avec les fichiers de données (`.jsonl`) de `out/`, mais regardent des choses différentes : **l'étape 6 vérifie la détection** – avec quelle fiabilité joueurs, arbitres et ballon ont été suivis. **L'étape 7 montre le match** – tentatives, possession et où se trouvaient les équipes. Regardez d'abord l'étape 6 : si ses valeurs sont mauvaises, les chiffres de l'étape 7 sont incertains eux aussi.

### Rapport de détection

Sous *Évaluation*, choisissez un fichier `.jsonl` et cliquez *Évaluer*. Le rapport apparaît dans la fenêtre de progression et dans le journal et répond à ceci :

| Ligne | Ce qui est bon signe |
|---|---|
| `exactement 5+5 dans …% des images` (basketball) | plus de 70 % |
| `Identifiants de joueurs distincts : domicile …, visiteurs …` | au plus environ 8 par équipe (davantage : les places sont souvent réattribuées) |
| `Visibles …%` | plus de 85 % |
| `pas à la vitesse limite` | moins de 3 % |
| `2 arbitres dans …%` | élevé si des arbitres sont sur le terrain |
| `Ballon : tenu / en vol / libre / inconnu` | la part « inconnu » indique combien de fois le ballon n'a pas été trouvé ; avec un modèle de ballon faible, elle peut être élevée |
| `Changements : …` | la liste des changements avec l'heure et les numéros |
| `Situations : …` | chaque situation reconnue avec l'heure, l'équipe et la confiance – comparez-en quelques-unes avec la vidéo |

![Étape 6](img/fr/step6.webp)
*Étape 6 : le rapport sur le fichier de données dans le journal.*

### Tableau tactique

Sous *Tableau tactique*, vous arrêtez un résultat à un *Instant* (p. ex. `12:30`) et ouvrez ce moment comme tableau tactique : tous les joueurs et le ballon se trouvent là où ils étaient à cet instant, *Trajet (s)* trace les dernières secondes de leurs déplacements. Sur le tableau, vous pouvez déplacer des joueurs, dessiner et enregistrer ; un tableau enregistré pour le même instant se rouvre tel que vous l'avez laissé. Plus de détails dans [Tactics board scenes](../README.md#tactics-board-scenes) (en anglais).


## Étape 7 · Statistiques

Choisissez en haut un fichier `.jsonl` dans `out/`. Les statistiques suivent le sport du calibrage (`court.json`) et ont besoin de ce calibrage. Ce sont des **tentatives** qui sont comptées – la détection ne voit pas si un tir est réussi. Chaque point, case et barre affiche ses valeurs exactes quand la souris reste dessus.

### Chiffres clés

Les tuiles en haut montrent les tentatives par équipe, la **possession** (part du temps où un joueur de l'équipe tient le ballon), *Ballon vu* (dans quel pourcentage des images le ballon a été trouvé) et la durée avec l'heure de début et de fin dans la vidéo. Si *Ballon vu* est faible, possession et tentatives ont des lacunes.

### Carte des tirs

Chaque tentative se trouve à son endroit sur le terrain, dans la couleur de l'équipe ; la forme indique le type (cercle 2 points ou tir, losange 3 points, triangle lancer franc ou 7 m). *Les deux / Domicile / Visiteurs* filtre, *Tableau* affiche les mêmes tentatives en liste avec heure, équipe, type, zone et distance.

* **Basket :** les tirs viennent de la détection des situations de jeu – elle doit avoir été activée lors du traitement (étape 4).
* **Sports de but :** une tentative est un vol rapide du ballon qui part à portée de tir et vise un but.

### Tentatives par zone

À gauche, une carte de chaleur des tentatives : le terrain est découpé en cases d'environ 2 × 2 m, plus une case est foncée, plus il y a eu de tentatives depuis là. À droite, la part de chaque zone en barre avec son nombre (basket : sous le panier, raquette, mi-distance, 3 pts du coin, 3 pts axe, lancer franc ; football : surface de but, surface de réparation, hors surface ; handball : 6 m, aile, arrière, 7 m ; futsal, hockey et crosse selon la distance). Le filtre *Les deux / Domicile / Visiteurs* s'applique ici aussi.

### Tentatives par section

La vidéo est divisée en sections de même durée (quatre au basket et au hockey, sinon deux) ; des barres par équipe et en dessous la différence domicile – visiteurs par section. Le programme ne connaît pas le chronomètre du match : les arrêts de jeu comptent, et une section n'est pas un quart-temps ou une mi-temps, sauf si la vidéo correspond exactement à une période.

### Possession et ballon sur le terrain

Une barre répartit le temps de possession entre domicile et visiteurs. Pour les sports de terrain, *Ballon sur le terrain* montre combien de temps le ballon était dans le tiers gauche, au milieu et dans le tiers droit – « gauche » et « droite » comme sur l'image de la caméra.

### Carte de chaleur des positions

La carte *Positions* est une carte de chaleur : elle montre où une équipe ou le ballon se trouvait pendant la vidéo.

![Étape 7 – carte de chaleur](img/fr/step7-heat.webp)
*Étape 7 : carte de chaleur des positions, avec en haut le choix Domicile / Visiteurs / Ballon.*

* **Basculer :** *Domicile*, *Visiteurs* ou *Ballon* choisit de qui les positions sont affichées. Les équipes apparaissent dans la couleur d'accent, le ballon en orange.
* **Comment elle est faite :** le terrain est divisé en 24 cases dans sa longueur et en un nombre adapté dans sa largeur. Pour chaque image, chaque case compte combien de joueurs de l'équipe s'y trouvaient (ou si le ballon y était). Seuls les joueurs **visibles** avec une équipe comptent – les positions estimées de joueurs cachés, les arbitres et les joueurs sans couleur d'équipe ne comptent pas. Le ballon ne compte que dans les images où il a été trouvé.
* **Comment la lire :** plus une case est intense, plus de temps y a été passé ; la case la plus intense est la plus fréquentée et les autres sont mesurées par rapport à elle. Les cases vides restent blanches. Laissez la souris sur une case pour voir sa part de toutes les positions comptées, en pourcentage.
* **À quoi elle sert :** où une équipe défend-elle, à quelle hauteur se place-t-elle, par quel côté attaque-t-elle, où le ballon passe-t-il le plus de temps ? Comparez *Domicile* et *Visiteurs* sur la même section, ou deux sections de la même équipe (p. ex. première et deuxième mi-temps en fichiers séparés).
* **Limites :** les joueurs éloignés de la caméra sont moins souvent détectés, le côté lointain paraît donc souvent plus pâle. Les changements de côté à la mi-temps se mélangent si les deux mi-temps sont dans un seul fichier. Les positions hors du terrain ne comptent pas.

### Moments clés et clips

La liste montre les tentatives, les situations de jeu et les changements confirmés avec heure, équipe et zone. Cochez des moments (*Tous* sélectionne tout) et cliquez *Couper les clips* : chacun donne un court clip de la vidéo dans `out/clips/`, de 6 s avant à 4 s après le moment. *Montrer les clips* ouvre le dossier. La vidéo de l'étape 2 doit toujours se trouver au même endroit.

---

## Mise à jour

**App Mac :** quand une nouvelle version est sortie, l'app demande au démarrage (en anglais) : *Version … is available. Install now?* Elle télécharge la mise à jour, vérifie sa signature, bascule et redémarre ; vos projets restent tels quels. Si une tâche est en cours, la mise à jour attend. À la main : menu *Reco Minimap → Check for Updates…*.

**Zip Windows et Linux :** télécharger le nouveau zip depuis la page des versions et le décompresser.

**Compilé vous-même (cargo) :** comme décrit ci-dessous.

Au démarrage, l'interface demande une fois à GitHub s'il existe une version plus récente. Seule la requête est envoyée : aucune vidéo, image ni donnée de projet.

* S'il y en a une, un **bandeau bleu** apparaît en haut avec le numéro de version, un lien *Quoi de neuf ?* et le bouton *Installer maintenant*.
* *Installer maintenant* récupère les sources de cette version et recompile le programme. Cela prend quelques minutes et nécessite la même installation de Rust que celle utilisée pour installer le programme. Le journal montre la progression.
* Arrêtez ensuite le programme dans le terminal avec **Ctrl+C** et relancez-le.

En ligne de commande : `reco-minimap update` indique s'il y a du nouveau ; `reco-minimap update --yes` l'installe. La vérification au démarrage se désactive avec `reco-minimap ui --no-update-check`.

## Utiliser les résultats

**Incruster la minimap dans une vidéo de match avec ffmpeg** (`segment1.mov` sur `match.mp4`, en bas à droite, 480 px de large) :

```bash
ffmpeg -i match.mp4 -i out/segment1.mov \
  -filter_complex "[1:v]scale=480:-1[m];[0:v][m]overlay=W-w-30:H-h-30:format=auto" \
  -c:v libx264 -crf 18 -pix_fmt yuv420p -c:a copy match_avec_minimap.mp4
```

L'overlay commence à l'instant de début du segment ; utilisez `-ss` sur la vidéo du match si votre segment ne commençait pas à zéro. Pour des matchs entiers en plusieurs segments, `scripts/run_game.sh` se charge du traitement et de l'incrustation.

**Les données** (`.jsonl`) servent à vos propres analyses : un objet JSON par image traitée, avec les positions en mètres dans le repère du terrain (`x` le long de la longueur depuis la gauche, `y` le long de la largeur depuis la ligne de touche du fond), l'état de chaque joueur (`visible`, `occluded`, `lost`), l'état du ballon (`held`, `flight`, `loose`, `unknown`) et les événements.

---

## En cas de problème

| Message ou symptôme | Que faire |
|---|---|
| Bibliothèque ONNX Runtime introuvable / bandeau rouge en haut | Faire pointer `ORT_DYLIB_PATH` sur `libonnxruntime…dylib` (version 1.24 ou plus récente) et lancer `reco-minimap ui` depuis le même terminal. Voir [Installation](#installation). |
| Le fournisseur CoreML n'est pas inclus | Prendre la bibliothèque du paquet Python (`pip install onnxruntime`), pas celle de Homebrew – ou régler *Calcul* sur `CPU`. |
| ffmpeg introuvable | `brew install ffmpeg` |
| *Parcourir …* ne fait rien | Les boîtes de dialogue de fichiers nécessitent macOS (ou `zenity` sous Linux). Collez le chemin à la place. |
| `zsh: command not found: #…` | Ne collez pas dans le terminal les lignes commençant par `#` – zsh ne les traite pas comme des commentaires. |
| *Détecter les couleurs* échoue | Vérifier le calibrage (seuls les joueurs dans le terrain calibré comptent) et régler les couleurs à la main. |
| Peu de joueurs détectés | Lancer *Mesurer* (3b). Le programme suit déjà un joueur avec des détections incertaines (à partir de 0,30) mais ne crée un point que sur une détection sûre (à partir de 0,60) ; si le modèle est incertain partout sur vos images, baissez `--thr-player` en ligne de commande (p. ex. 0,5). Une image petite ou floue limite le modèle. |
| Ballon rarement trouvé (modèle par défaut) | Le modèle général de Reco camera est moins sûr pour le ballon. Importer un modèle entraîné dans Reco Trainer pour le sport et le choisir à l'étape 1. Avec le modèle par défaut, des seuils plus bas s'appliquent (joueurs à partir de 0,35, ballon à partir de 0,20). |
| *Télécharger le modèle* échoue | Il faut une fois une connexion à GitHub. Si vous avez déjà `yolo26n.onnx` (p. ex. de Reco camera) : `reco-minimap fetch-model --from /chemin/vers/yolo26n.onnx`. |
| Couleurs d'équipe fausses, ou points de la mauvaise couleur | Régler les couleurs à la main en 3c. Pour des couleurs vives ou des zones de terrain colorées, activer *Rapprocher l'échantillon des couleurs de maillot*. |
| Deux « arbitres » alors qu'il n'y en a pas | Activer *Match sans arbitres* en 3c. |
| Points au mauvais endroit | Recalibrer avec plus de points sur tout le terrain ; le terrain vert doit se superposer aux vraies lignes. |
| Arbitres absents | Régler la couleur du maillot des arbitres (3c). |
| Très lent | *Détecter toutes les N images* avec un N plus grand ; *Calcul* sur CoreML ; lancer *Mesurer* pour réduire le nombre de tuiles. |
| Les numéros restent vides | Le modèle de numéros est-il installé ? Les joueurs sont-ils assez grands (récapitulatif en fin de traitement) ? Avez-vous saisi les effectifs ? |
| Port occupé | Lancer avec `reco-minimap ui --port 8800`. |

Bloqué ? Regardez le journal (en bas de la page) : il montre la commande exacte (`$ reco-minimap …`). Vous pouvez la copier dans un terminal et obtenir la même sortie avec plus de détails.
