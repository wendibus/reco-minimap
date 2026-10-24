# Fil en direct – fonctionnement et mise en place

*Dans d'autres langues : [English](TICKER.md) · [Deutsch](TICKER.de.md) · [Español](TICKER.es.md)*

![Fonctionnement du fil en direct](img/ticker-flow.fr.svg)

## 1. De quoi s'agit-il

Pendant que `reco-minimap live` analyse la vidéo panoramique, il repère les **changements de joueurs** et les **situations de jeu** (coup d'envoi, touche, corner, penalty, lancer franc, tentatives de tir – selon le sport). Le fil en fait un court flux de texte, le plus récent en haut, avec le temps de la vidéo :

> 3:01 Changement Sharks : #7 sort, #12 entre
> 1:35 Tir à 3 points → Sharks (près de la ligne)

Il fonctionne pour tous les sports et s'affiche en allemand, anglais, français et espagnol (chaque visiteur choisit sa langue). Vous pouvez l'afficher sur les téléphones dans la salle, l'intégrer au site de votre club et décider s'il montre des **numéros**, des **noms** ou **les deux** – et si chaque entrée attend d'abord **votre accord**.

## 2. Fonctionnement

Suivez l'image depuis le haut à gauche :

1. **Caméra → `reco-minimap live`.** Le programme lit la vidéo panoramique et détecte joueurs, changements et situations de jeu. Rien n'est encore envoyé.
2. **Validation (facultatif).** Avec la *validation manuelle*, chaque nouvelle entrée attend dans une file. Vous ouvrez une page qui ne fonctionne **que sur votre ordinateur** et cliquez sur *Valider* ou *Rejeter* pour chaque entrée. Sans validation, les entrées sont publiées aussitôt.
3. **Fil public.** Les entrées validées forment le fil. Les noms n'en font partie que si vous les autorisez ; avec *numéros*, ils ne quittent jamais le programme.
4. **Deux façons de l'afficher :**
   * **Téléphones sur le même Wi-Fi** ouvrent `http://<votre-ordinateur>:8765/ticker`. Simple, sans hébergement.
   * **Votre site web.** L'*émetteur* écrit le fil dans un petit fichier, `ticker.json`, et le téléverse sur votre hébergement (SFTP / FTP / WebDAV).
5. **Votre hébergement** ne contient que des **fichiers statiques** (`ticker.json`, `ticker.js`, `ticker.html`). Rien à installer sur le serveur : ni PHP, ni base de données, ni connexion.
6. **Les visiteurs** ouvrent votre page habituelle. Un petit widget JavaScript (ou un iframe) y lit `ticker.json` toutes les 3 secondes et dessine le fil. Un script dans le navigateur d'un visiteur ne peut que *lire* ; c'est pourquoi le programme se charge de l'envoi.

Les positions, l'overlay en direct et la page de validation restent sur votre ordinateur. Seules les entrées validées arrivent sur l'hébergement.

## 3. Ce qu'il vous faut

* `reco-minimap` **0.8.0 ou plus récent** ([installation](../README.md#install) ; mise à jour avec `reco-minimap update --yes`). Vérifier avec `reco-minimap --version`.
* Une configuration live qui fonctionne : modèle, `court.json` et `teams.json` (voir le [guide](WALKTHROUGH.fr.md)).
* Les **situations de jeu** demandent `--scenes`, les **changements** demandent les numéros de maillot (`--numbers`, idéalement avec les effectifs).
* Pour votre site : un hébergement avec **accès SFTP ou FTP** (presque tous en ont un) et le programme `curl` (déjà présent sur macOS, Linux et Windows 10/11 ; vérifier avec `curl --version`).

## 4. Mise en place – choisissez votre niveau

### A. Un simple lien (5 minutes, sans hébergement)

Lancez live comme d'habitude avec les situations et les numéros. Dans l'interface, c'est l'étape **5 · Direct** (la case *Détecter les situations de jeu* se trouve à l'étape 4 · Traitement et vaut aussi pour le direct) ; en ligne de commande :

```bash
reco-minimap live <source> --model <dossier-modèles> --court court.json --teams teams.json \
    --size 1920x1080 --scenes --numbers
```

Le fil est à l'adresse **`http://127.0.0.1:8765/ticker`**. Pour des téléphones sur le même Wi-Fi, ajoutez `--host 0.0.0.0` (interface : *Accessible depuis d'autres appareils du réseau*) et envoyez `http://<adresse-de-votre-ordinateur>:8765/ticker`. L'interface affiche le lien tout prêt ; en ligne de commande, trouvez l'adresse avec `ipconfig` (Windows), `hostname -I` (Linux) ou *Réglages Système → Wi-Fi → Détails* (macOS). Le bouton *Partager* de la page transmet le lien au menu de partage du téléphone.

> `--host 0.0.0.0` rend aussi l'overlay en direct et les positions accessibles à tout le réseau. À utiliser sur des réseaux de confiance (pas le Wi-Fi invité public d'une salle).

### B. Sur votre propre site web

1. **Récupérez vos accès** chez votre hébergeur : nom du serveur, utilisateur, mot de passe et dossier de votre site (souvent `htdocs` ou `public_html`). Préférez **SFTP** (`sftp://`) ou `ftps://` ; `ftp://` simple envoie le mot de passe en clair.
2. **Créez les fichiers une fois.** Lancez live (ou un test, voir plus bas) avec `--ticker-export out/ticker`, ou cochez *Exporter pour un site web* dans l'interface. Le dossier `out/ticker` contient alors `ticker.html`, `ticker.js`, `index.html`, `.htaccess` et, dès que live tourne, `ticker.json`.
3. **Téléversez le dossier une fois** dans un dossier de votre hébergement, par exemple `htdocs/ticker/`, avec n'importe quel programme FTP (FileZilla, Cyberduck, WinSCP). Gardez tous les fichiers, y compris `.htaccess`.
4. **Regardez le résultat :** ouvrez `https://votre-domaine/ticker/index.html`. Elle montre les extraits à copier et un widget de test (il indique *hors ligne* tant qu'aucun match ne tourne).
5. **Intégrez-le** dans votre page (section 5), par exemple dans un bloc *HTML personnalisé*.
6. **Le jour du match**, laissez le programme téléverser `ticker.json` lui-même :

   ```bash
   reco-minimap live <source> --model <dossier-modèles> --court court.json --teams teams.json \
       --size 1920x1080 --scenes --numbers \
       --ticker-export out/ticker \
       --ticker-push "sftp://UTILISATEUR:MOTDEPASSE@SERVEUR/htdocs/ticker/ticker.json"
   ```

   Dans l'interface : *Exporter pour un site web* et, dans *Téléverser automatiquement vers*, la même adresse. Le programme n'écrase que ce fichier, d'abord sous un nom temporaire puis renommé : personne ne lit jamais un fichier à moitié écrit. Les dossiers manquants sont créés.

   Les caractères spéciaux du mot de passe s'écrivent avec codage pourcent : `@` → `%40`, `:` → `%3A`, `/` → `%2F`, `#` → `%23`, `%` → `%25`, espace → `%20`.

**Autres destinations :** WebDAV (Nextcloud, IONOS HiDrive …), S3/R2 ou un Cloudflare Worker conviennent aussi : `--ticker-push https://…/ticker.json [--ticker-key TOKEN]` envoie le fichier par HTTP `PUT` (`--ticker-push-method POST` si besoin). Si vous préférez synchroniser le dossier vous-même (rsync, Syncthing, Cloudflare Pages, GitHub Pages), n'utilisez que `--ticker-export`.

**Test sans match :** lisez un match enregistré comme s'il était en direct :

```bash
reco-minimap live match.mp4 --realtime --size 1920x1080 --model … --court court.json --teams teams.json --scenes --numbers --ticker-export out/ticker --ticker-push "sftp://…"
```

### C. Avec votre accord avant toute publication

Ajoutez `--ticker-approve` (interface : *Validation manuelle*). Ouvrez **`http://127.0.0.1:8765/ticker/admin`** sur le même ordinateur. Les entrées en attente sont en haut : *Valider* ou *Rejeter* chacune, ou *Tout valider*. Une entrée publiée peut être *retirée* (chez les visiteurs elle disparaît à l'actualisation suivante). L'interrupteur *Validation manuelle* active et désactive la file pendant le match. La page ne fonctionne que depuis cet ordinateur – ni par Wi-Fi, ni depuis d'autres pages web. Elle affiche aussi les noms quand le fil public est réglé sur les numéros, pour que vous voyiez qui est concerné. Les entrées rejetées n'atteignent ni le fil ni l'hébergement.

## 5. Intégration

**iframe – partout où l'on peut coller du HTML :**

```html
<iframe src="https://votre-domaine/ticker/ticker.html?embed=1&lang=fr"
        style="width:100%;height:480px;border:0" loading="lazy"></iframe>
```

| Option dans l'adresse | Signification |
|---|---|
| `embed=1` | uniquement la liste (sans en-tête ni choix de langue) – pour l'intégration |
| `lang=de` `en` `fr` `es` | langue (sans indication : langue du navigateur du visiteur) |
| `show=numbers` `names` `both` | affichage (dans la limite des noms que vous autorisez) |
| `theme=dark` `light` | couleurs (par défaut : selon l'appareil du visiteur) |
| `max=20` | au plus 20 entrées |

Pour que le cadre ait la hauteur de son contenu, ajoutez ceci une fois à votre page :

```html
<script>addEventListener('message',function(e){if(e.data&&e.data.type==='reco-ticker-height'){document.querySelector('iframe[src*="ticker.html"]').style.height=e.data.height+'px'}})</script>
```

**Widget – reprend la police et le style de votre page (fichiers sur le même domaine) :**

```html
<script src="https://votre-domaine/ticker/ticker.js"
        data-feed="https://votre-domaine/ticker/ticker.json" data-lang="fr" data-max="20"></script>
```

ou comme élément : `<reco-ticker feed="…/ticker.json" lang="fr" max="20" theme="light"></reco-ticker>`. Les couleurs se changent avec des variables CSS sur l'élément :

```css
reco-ticker { --rt-home:#c8102e; --rt-away:#005bbb; --rt-acc:#c8102e;
              --rt-card:#fff; --rt-fg:#222; --rt-line:#ddd; --rt-dim:#666; }
```

Si les fichiers sont sur un *autre* domaine que votre page, ce serveur doit autoriser la lecture de `ticker.json` depuis votre page (CORS) ; le `.htaccess` fourni le fait sur Apache. L'iframe n'a pas besoin de ce réglage.

## 6. Noms, numéros et protection des données

`--ticker-names numbers|names|both` définit ce que voit le public ; par défaut **numbers**.

* **numbers :** `#7`. Les noms ne sont **envoyés à aucun appareil**, pas même dans `ticker.json`. C'est le réglage sûr, surtout pour les équipes de jeunes.
* **names / both :** la page propose un interrupteur *Numéros · Noms · Les deux* ; chaque visiteur peut afficher **moins** que ce que vous autorisez, jamais plus. `show=numbers` dans un lien le préréglage.

Les noms viennent des listes d'effectif, écrits après le numéro avec `:` ou `=` : `--roster-home "7:Anna Müller, 12:Ben, 1g:Tom"` (la marque de gardien fonctionne toujours ; les noms peuvent contenir des espaces, pas de virgules). Dans l'interface, utilisez les champs d'effectif de *Numéros de maillot* (à l'étape Direct). Quelqu'un sans nom apparaît comme `#7` ; un numéro inconnu comme *un joueur*. `--home-name` et `--away-name` définissent les noms d'équipe dans l'en-tête.

Demandez-vous si les noms de mineurs ont leur place sur une page publique, et obtenez l'accord des parents et du club avant de passer à *names* ou *both*.

## 7. Liste de contrôle du jour de match

1. Ordinateur sur secteur, sans mise en veille ; connexion internet si vous téléversez.
2. Lancez `live` environ 10 minutes avant le début ; surveillez les messages *Envoi du fil* (un envoi échoué est réessayé et signalé une fois, puis de nouveau quand il refonctionne).
3. Ouvrez votre page sur un téléphone : le statut indique **en direct**. Après environ 45 secondes sans signe de vie, il indique **pas de connexion**.
4. Avec validation manuelle : gardez `…/ticker/admin` ouvert sur l'ordinateur.
5. Après le match, arrêtez le programme. La page garde les dernières entrées ; relancer `live` commence un fil vide et écrase `ticker.json`.

## 8. Aperçu des options

| Ligne de commande | Interface (étape 5 · Direct) | Défaut | Signification |
|---|---|---|---|
| `--scenes` | Détecter les situations de jeu (étape 4 · Traitement) | non | détecter remises en jeu et tentatives de tir |
| `--numbers` | Numéros de maillot (aussi à l'étape Direct) | non | lire les numéros (nécessaire pour les changements) |
| `--roster-home/away "7:Anna, 12:Ben"` | champs d'effectif | – | numéros de l'effectif, noms facultatifs |
| `--ticker-names numbers\|names\|both` | Affichage | `numbers` | ce que le public peut voir |
| `--home-name` / `--away-name` | Nom domicile / visiteurs | – | noms d'équipe dans l'en-tête |
| `--ticker-approve` | Validation manuelle | non | les entrées attendent votre accord |
| `--host 0.0.0.0` | Accessible depuis d'autres appareils | non | téléphones sur le même Wi-Fi |
| `--ticker-export DOSSIER` | Exporter pour un site web | non | écrire les fichiers statiques dans un dossier |
| `--ticker-push URL` | Téléverser automatiquement vers | non | `sftp://`, `ftps://`, `ftp://`, `https://` (WebDAV, S3, Worker) |
| `--ticker-key TOKEN` | Clé | – | envoyée comme `Authorization: Bearer` (http/https seulement) |
| `--ticker-push-method` | – | `PUT` | `PUT` ou `POST` (http/https seulement) |

## 9. En cas de problème

| Problème | Cause probable et solution |
|---|---|
| La page indique **pas de connexion** | Le programme ne tourne pas, ou l'envoi échoue. Cherchez *Échec de l'envoi du fil (…)* dans la console du programme. Vérifiez l'adresse, le mot de passe (codage pourcent !) et le dossier. |
| **Rien n'apparaît** | Rien n'a encore été détecté. Les situations demandent `--scenes`, les changements `--numbers`. Avec validation manuelle, les entrées attendent dans `…/ticker/admin`. |
| **Les noms n'apparaissent pas** | `--ticker-names` est encore sur `numbers`, ou la liste d'effectif n'a pas d'entrées comme `7:Nom`. |
| **Les téléphones n'atteignent pas le lien** | `--host 0.0.0.0` manquant, un pare-feu, ou un Wi-Fi qui isole les appareils (réseaux invités). |
| **Le widget reste vide** | Fichiers sur un autre domaine : CORS (utilisez le `.htaccess` ou l'iframe). Vérifiez que `ticker.json` s'ouvre dans le navigateur. |
| **Anciennes entrées après rechargement** | Un cache intermédiaire. La page demande un fichier frais à chaque fois et `.htaccess` règle `no-store` ; sur d'autres serveurs, désactivez le cache pour `*.json`. |
| `--ticker-push nécessite le programme curl` | Installez curl, ou n'utilisez que `--ticker-export` et synchronisez le dossier vous-même. |
| Des caractères spéciaux cassent l'envoi | Écrivez-les avec codage pourcent dans l'adresse (section 4 B). |

## 10. Limites

* Le fil repose sur une **détection automatique** à partir de l'image de la caméra. Il peut manquer des choses, arriver en retard ou se tromper ; un changement *possible* est marqué *non confirmé*. Ce n'est pas un rapport d'arbitre et il **n'affiche pas le score**.
* Le temps dans le fil est le **temps de la vidéo**.
* Le fil garde les 600 dernières entrées ; un visiteur qui arrive en retard les voit toutes.
* Sans serveur web propre, un lien sur votre ordinateur n'est joignable de l'extérieur de votre réseau que via un tunnel ou un service que vous installez vous-même – le hub live n'a pas de connexion. C'est pourquoi la voie « site web » téléverse un fichier.
