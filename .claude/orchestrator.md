# Briefing orchestrateur — alcoLoco

Application de suivi d'alcoolémie. Backend **Rust / axum**, frontend **Angular**,
base **PostgreSQL**. Dépôt public `maelprog/alcoLoco`, branche par défaut `main`.

> **État au 2026-08-18 : socle en place.** #1 est mergée (PR #39, `5a9f96f`). `main`
> porte le workspace Cargo (`crates/api` axum + `crates/domain`), l'app Angular dans
> `web/`, `docker-compose.yml` (Postgres 16) et le workflow CI. La section *Gates*
> ci-dessous n'est plus provisoire : elle recopie `.github/workflows/ci.yml`.

---

## Environnement

**Il n'y a aucune toolchain sur l'hôte.** Vérifié : `cargo`, `rustc`, `rustup`,
`node`, `npm`, `psql` sont **absents**. Seuls `docker` (démon actif), `git` et `gh`
sont disponibles.

⚠ **`docker compose` n'existe pas non plus** — corrigé le 2026-08-19, après qu'un
agent s'y soit cassé les dents. Ce briefing annonçait un `docker compose` v5.4.0 :
c'était faux. Vérifié deux fois, par le vérificateur de #2 puis par l'orchestrateur :

```
$ docker compose version     → docker: unknown command: docker compose
$ docker-compose version     → could not be found in this WSL 2 distro
```

Ne cherche pas à l'installer. Postgres se lance en `docker run` direct, cf. plus bas.

**Tout gate passe donc par un conteneur.** N'essaie pas d'installer une
toolchain sur l'hôte, et ne rapporte jamais un gate comme non exécutable au
motif que `cargo` est introuvable — c'est attendu.

### Image Rust — `alcoloco-rust:dev`

⚠ **Ni `rust:1-bookworm` ni `rust:1-slim-bookworm` ne contiennent `rustfmt` ni
`clippy`** (vérifié : les deux échouent sur `cargo fmt --version`). Une image
dérivée est déjà construite et disponible localement :

```
alcoloco-rust:dev   # rust 1.97.1 + rustfmt 1.9.0 + clippy 0.1.97
```

Si elle a disparu, reconstruis-la (≈ 8 s) :

```bash
printf 'FROM rust:1-bookworm\nRUN rustup component add rustfmt clippy\n' > /tmp/Dockerfile.rustdev
docker build -f /tmp/Dockerfile.rustdev -t alcoloco-rust:dev /tmp
```

### Recette d'exécution

Depuis la racine du worktree. Le cache `cargo` est monté sur un volume nommé
**partagé entre agents** : c'est voulu (compilations bien plus rapides), et sans
risque tant que personne ne lance deux `cargo` concurrents sur le même crate.

```bash
docker run --rm -v "$PWD":/w -w /w \
  -v alcoloco-cargo:/usr/local/cargo/registry \
  alcoloco-rust:dev <commande cargo>
```

Frontend — image `node:22-bookworm-slim` (présente localement, node 22.23.2 /
npm 10.9.8) :

```bash
docker run --rm -v "$PWD/web":/w -w /w node:22-bookworm-slim <commande npm/ng>
```

### Postgres

`docker-compose.yml` est sur `main` depuis #1 : service `db`, image
**`postgres:16-alpine`**, healthcheck `pg_isready`, volume nommé `db_data`. Il lit
quatre variables, avec défauts : `POSTGRES_USER`/`POSTGRES_PASSWORD`/`POSTGRES_DB`
= `alcoloco`, et `POSTGRES_PORT` = `5432`.

⚠ **Mais `docker compose` est indisponible** (cf. *Environnement*) : ce fichier
décrit le service et la documentation. **On ne peut pas le lancer sur cette machine.**
⚠ Il n'est **pas** exécuté par la CI (aucune étape `compose` dans `ci.yml`) mais il **est**
lu par un test : `crates/db/src/lib.rs` fait `include_str!` dessus et compare ses défauts à
`DEFAULT_DATABASE_URL`. En changer une valeur casse `cargo test --workspace`. Lance Postgres en `docker run` direct, en reprenant les
mêmes valeurs, avec **ton propre nom de conteneur et ton propre port hôte** :

```bash
# agent A (implémentation) : nom alcoloco-<n>,   port 54<nn>
# agent B (vérification)   : nom alcoloco-<n>-b, port 55<nn>
docker run -d --name alcoloco-<n> \
  -e POSTGRES_USER=alcoloco -e POSTGRES_PASSWORD=alcoloco -e POSTGRES_DB=alcoloco \
  -p 54<n sur 2 chiffres>:5432 postgres:16-alpine
# prêt quand (via TCP, pas la socket unix — cf. ci-dessous) :
#   docker exec alcoloco-<n> pg_isready -h 127.0.0.1 -p 5432
# DATABASE_URL=postgres://alcoloco:alcoloco@localhost:54<nn>/alcoloco
```

Le **nom** et le **port** dérivent tous deux du numéro d'issue : deux agents sur la
même issue se collisionneraient (`Conflict. The container name … is already in use`),
et le nettoyage de A détruirait la base de B en pleine vérification. D'où les deux
gammes ci-dessus — **A prend `alcoloco-<n>` / `54<nn>`, B prend `alcoloco-<n>-b` /
`55<nn>`**, sans négociation.

⚠ `pg_isready -U alcoloco` **sur la socket unix répond « accepting connections »
pendant `initdb`**, avant que le port TCP n'écoute : l'entrypoint postgres lance un
serveur temporaire en `listen_addresses=''`. Sonder en TCP comme ci-dessus, ou
attendre la **deuxième** occurrence de « database system is ready to accept
connections » dans `docker logs`.

Depuis le conteneur Rust, ajoute `--network host` au `docker run` pour joindre
`localhost:54<nn>`.

Arrêt et nettoyage en fin d'issue : **`docker rm -fv alcoloco-<n>`**. Le `-v` n'est pas
facultatif : `postgres:16-alpine` déclare `VOLUME /var/lib/postgresql/data`, donc chaque
`docker run` crée un volume **anonyme** que `docker rm -f` seul laisse derrière lui.
(Constaté le 2026-08-20 : 29 volumes orphelins, 2,4 Go récupérables.)

---

## Gates

> **Recopié de `.github/workflows/ci.yml` le 2026-08-18** (workflow `CI`, sur
> `pull_request` et sur `push` vers `main`). Deux jobs, tous deux `ubuntu-latest` :
> **`rust`** (gates 1 à 3) et **`web`** (gates 4 et 5, `working-directory: web`).
> Ce sont les deux checks à attendre verts sur une PR.
>
> La CI épingle sa toolchain par variable d'environnement du workflow —
> `RUST_TOOLCHAIN: 1.97.1` et `NODE_VERSION: 22` — et **il n'y a pas de
> `rust-toolchain.toml`** : les images `alcoloco-rust:dev` (rust 1.97.1) et
> `node:22-bookworm-slim` de la section *Environnement* sont donc alignées sur la
> CI, mais rien ne le garantit mécaniquement. Toute bascule de version se fait des
> deux côtés à la fois.

Dans l'ordre, tous doivent être verts avant d'ouvrir la PR :

| # | Gate | Job CI | Commande (dans le conteneur, cf. *Environnement*) |
|---|---|---|---|
| 1 | Format Rust | `rust` | `cargo fmt --all -- --check` |
| 2 | Lint Rust | `rust` | `cargo clippy --all-targets --all-features -- -D warnings` |
| 3 | Tests Rust | `rust` | `cargo test --workspace` |
| 4 | Lint front | `web` | `npm run lint` (dans `web/`, → `ng lint`) |
| 5 | Tests front | `web` | `npm run test -- --watch=false` (dans `web/`, → `ng test`) |

⚠ **Gate 5 : plus de `--browsers=ChromeHeadless`.** Angular 22 utilise le lanceur
**vitest + jsdom** par défaut ; l'ancienne forme Karma échoue. Aucun navigateur
n'est nécessaire dans le conteneur. La CI lance exactement `npm run test -- --watch=false`.

Les gates 4 et 5 ne s'appliquent qu'aux issues touchant `web/`, les gates 1 à 3
qu'à celles touchant du Rust. Une issue purement backend ne lance pas les gates
front — mais elle le **dit** dans la PR, elle ne les passe pas sous silence.

**Un gate non exécuté n'est jamais « probablement vert ».** Rapporte la commande
lancée et sa sortie, ou déclare le gate non applicable avec son motif.

---

## Sources de vérité

- **`SPEC.md`** — source de vérité **unique** pour le périmètre, le modèle de
  domaine et les règles de calcul. À lire selon la zone touchée, pas en entier :
  - modèle de données / migrations → §4, §7 ;
  - moteur de calcul → §6 en entier, puis §10.0 et §10.1 ;
  - écrans et parcours → §5 ;
  - découpage en modules → §9.
- **`README.md`** — résumé d'entrée, sans autorité. En cas de contradiction avec
  `SPEC.md`, c'est `SPEC.md` qui gagne, et l'écart se signale dans la PR.
- Le **corps de l'issue** fait autorité sur son propre périmètre et porte ses
  critères d'acceptation.

⚠ **Piège de nommage** : toutes les issues renvoient à `SPEC_CORRECTED.md`.
**Ce fichier n'existe pas** — il s'agit de `SPEC.md`. Les numéros de section
cités sont justes. Ne pars pas chercher un document manquant.

Il n'y a **pas de `CLAUDE.md`** dans ce dépôt.

### Arbitrages du 2026-08-17 et du 2026-08-19 — appliqués à `SPEC.md`, à ne pas rouvrir

Ces points étaient ouverts ou faux dans la spec initiale. Ils sont désormais tranchés et écrits en
§10.0. Un agent qui croit devoir les rediscuter se trompe : il doit les appliquer.

| Réf. | Décision |
|---|---|
| §6.2 | Coefficient d'âge de Watson = **`0,09516`**, valeur de l'article d'origine (*Am J Clin Nutr* 1980;33:27-39). Une variante `0,09156` circule sur des calculateurs en ligne : **c'est elle qui est fausse**, ne pas l'introduire. |
| §10.0-K | L'**âge** est calculé à l'**heure d'ingestion** de la boisson, depuis la date de naissance de la version en vigueur. Aucun âge n'est stocké. |
| §10.0-H | **`β` est une constante** du crate `domain` (0,15 g/L/h), paramétrable en argument de fonction pour les tests. **Pas de colonne en base, pas de champ d'API.** |
| §10.0-I | Pas d'intégration interne **fixe à 1 min** ; fenêtre = durée de l'événement **+ 3 h**. Le `?step=` de #17 **sous-échantillonne seulement**, borné à **[1 min, 1 h]**, hors bornes → 400. |
| §10.0-J | `valid_from` d'une version de paramètres est **fourni par l'utilisateur** (défaut : maintenant), peut être rétroactif, jamais futur. Versions sans chevauchement ni trou ; la plus ancienne a une borne basse ouverte. Les **préférences de saisie ne sont pas versionnées**. |
| §4 / §5.1 | **Emplacement des paramètres physiologiques — arbitré par l'utilisateur le 2026-08-19.** Poids, taille, sexe et date de naissance vivent **uniquement** dans `profile_settings_version`. `profile` ne porte que l'identité et les préférences de saisie, et **aucune copie courante** : ce serait une seconde source de vérité libre de diverger dès la première modification. Les valeurs courantes se lisent par `profile_settings_at(id, now())`. Motif décisif : §10.0-K impose déjà au moteur d'utiliser la version en vigueur à l'heure d'ingestion — une copie sur `profile` ne serait lue par aucun calcul. L'obligation de §10.0-D reste tenue **hors concurrence**, par les triggers de contrainte différés du schéma ; **sous concurrence elle ne l'est pas** — c'est la classe (d) de `SPEC.md` §10.0-L, et **#43** porte la fermeture réelle. Ce qui n'est pas rouvrable ici, c'est l'**emplacement** des paramètres ; la portée de l'invariant, elle, est celle que §10.0-L énonce. **Ne pas ajouter ces colonnes à `profile`.** |
| §10.1 | Le tableau des variantes d'absorption a été recalculé : le trapèze pique à **0,227 g/L à 45,7 min**, et non aux valeurs de la rampe linéaire comme l'indiquait la spec initiale. Ce sont ces valeurs qui font foi pour les tests de #5, #16 et #37. |

⚠ Les corps des issues **#5, #16 et #37** recopient l'ancien coefficient et l'ancien tableau. Tant
qu'ils ne sont pas mis à jour, **c'est `SPEC.md` qui fait foi** — le signaler dans la PR concernée.

---

## Conventions d'API — à poser dans #3, valables partout ensuite

Tranchées par l'orchestrateur (points routiniers, sans enjeu produit). Elles existent pour que #6,
#11, #14, #15, #17 et #18 n'inventent pas chacune la leur. **#3 les pose une fois** ; les issues
suivantes les réutilisent sans les rediscuter.

- **Identifiants** : `uuid` en base, **UUID v7 généré côté application** (crate `uuid`, feature
  `v7`). Postgres 16 n'a pas de `uuidv7()` natif — ne pas le chercher. V7 est ordonné dans le temps,
  ce qui le rend directement utilisable comme curseur de pagination.
- **Erreurs** : **RFC 7807** `application/problem+json`, champs `type`, `title`, `status`, `detail`,
  plus un `errors[]` pour les échecs de validation champ par champ. Une erreur métier et une erreur
  interne partagent la même forme — c'est un critère d'acceptation de #3.
- **Pagination** : **par curseur**, jamais par offset. #15 exige une pagination « stable en cas
  d'insertion concurrente », ce qui exclut `OFFSET`. Contrat : `?limit=&cursor=`, réponse
  `{ "items": [...], "next_cursor": <string|null> }`, `limit` par défaut 50, plafond 200.
- **Dates** : `timestamptz` en base, ISO 8601 UTC avec suffixe `Z` sur le fil. Aucune date locale
  ne traverse l'API ; la conversion en fuseau local est un travail de front (§7).
- **Nommage** : `snake_case` dans les payloads JSON, aligné sur les colonnes.
- **Nom auto-proposé d'une boisson** (§5.3 mode 2) : la règle vit dans le crate `domain` et **n'est
  pas dupliquée côté front**. #13 l'implémente et #14 l'expose ; #20 l'appelle à chaque changement
  de la liste des composantes. **Départage en cas d'égalité de volume : l'ordre d'ajout** (la
  composante saisie en premier gagne) — sans quoi le critère d'acceptation de #13 n'est pas
  déterministe.
- **`volume_total` d'une boisson** : obligatoire si l'unité est `%`, **dérivé** de la somme des
  composantes si l'unité est `cL` — jamais saisi deux fois, jamais stocké en contradiction.

## Tests

Pas de TDD imposé, mais le moteur de calcul est l'exception explicite : §7 de la
spec exige qu'il soit « isolé du reste du backend (module dédié, testable
unitairement) », et §10.1 impose que le profil d'absorption soit derrière une
**abstraction remplaçable** (`AbsorptionProfile` dans le crate `domain`).

Seul modèle existant à ce jour : les deux tests unitaires de
`crates/domain/src/lib.rs` (module `#[cfg(test)] mod tests`, comparaison de flottants
par `assert!((x - attendu).abs() < 1e-9)`). Les modèles suivants viendront de #3
(tests d'intégration API) et #5 / #16 (cas de référence du moteur).
`TODO` : citer des fichiers réels une fois #3 et #16 mergées.

⚠ **#1 a déjà posé un morceau du périmètre de #16** : `crates/domain` porte
`ETHANOL_DENSITY_G_PER_ML` (0,789) et `ingested_alcohol_grams(volume_ml, abv_percent)`,
conformes à §6.1 — formule et constante vérifiées le 2026-08-18, ainsi que le cas de
test (250 mL à 5 % → 9,8625 g). **#16 les réutilise, ne les réécrit pas** et ne
duplique pas la constante ailleurs.

Contrainte forte issue de #5, à ne pas contourner : les cas de référence qui
dépendent du modèle d'absorption sont écrits **marqués `#[ignore]`** :

```rust
#[ignore = "modèle d'absorption non arbitré — cf. #37"]
```

**Retirer un `#[ignore]` de cette famille est l'acte qui clôt #37** — donc
interdit à un agent sans arbitrage utilisateur explicite. À l'inverse, ajouter
un `#[ignore]` pour faire passer la CI est interdit dans tous les cas.

---

## Dépendances

**Il n'y a aucun audit de sécurité en CI** : `cargo audit` et `npm audit` ont été
**écartés** du workflow par #1 (constaté dans `.github/workflows/ci.yml` le
2026-08-18 — les jobs `rust` et `web` s'arrêtent au format, au lint et aux tests).
Rien ne contrôle donc les CVE des dépendances aujourd'hui. C'est une dette assumée,
pas un oubli à corriger au passage : l'ajouter modifie la CI, ce qui relève de
l'*Escalade*.

- Pas d'ajout de dépendance qui ne serve pas directement l'issue en cours ; toute
  nouvelle dépendance est justifiée en une ligne dans le corps de la PR.
- Versions épinglées via `Cargo.lock` / `package-lock.json`, tous deux **commités**.
- **Interdit pour faire passer la CI** : ajouter un ignore d'audit de sécurité,
  désactiver une règle clippy en `allow` à l'échelle d'un crate, ou relâcher une
  contrainte de version.
- Le workspace est en **`edition = "2024"` / `resolver = "3"`** (posé par #1, hors
  spec). Les crates ajoutés ensuite héritent de `[workspace.package]` et ne
  redéfinissent pas leur édition.

---

## Conventions

> Arbitrées par l'utilisateur le 2026-08-17. Le dépôt n'a qu'un seul commit
> (`Spécification initiale du projet`) et ne fixait aucune convention.

- **Branche** : `feat/<n>-<slug>`, `fix/<n>-<slug>`, `chore/<n>-<slug>`,
  **slug en anglais**. Ex. `feat/1-bootstrap-repo`.
- **Commits** : Conventional Commits, **sujet en anglais**, à l'impératif.
  Ex. `feat(domain): add trapezoidal absorption profile`.
- **Trailers interdits** : pas de `Co-Authored-By`, pas de mention d'outil
  d'assistance ni d'IA, dans les commits comme dans les corps de PR.
- **Corps de PR** : en français. `Closes #<n>`, puis ce qui a été fait, puis le
  tableau des gates avec leur sortie réelle, puis les points signalés à la
  vérification.
- **Langues** — répartition à retenir :
  - `SPEC.md`, `README.md`, issues, corps de PR, doc utilisateur → **français** ;
  - branches, sujets de commit, identifiants de code et commentaires → **anglais**.

  Arbitré le 2026-08-17 : le code est en anglais. Les noms de tables et de
  colonnes suivent la même règle (`profile_settings_version`, `drink_component`),
  ce que le schéma de #2 applique déjà.

---

## Pièges connus

Section vivante — une ligne à ajouter après chaque mur rencontré.

- **1.** **`rustfmt` / `clippy` absents des images Rust officielles** (`rust:1-bookworm`
   *et* `rust:1-slim-bookworm`). Utiliser `alcoloco-rust:dev`, cf. *Environnement*.
- **2.** **Aucune toolchain hôte.** `cargo`/`node`/`npm`/`psql` sont introuvables et le
   resteront : c'est la configuration normale de cette machine, pas une panne.
- **3.** **`SPEC_CORRECTED.md` n'existe pas** — lire `SPEC.md` (les §§ cités sont bons).
- **4.** **Collision de port Postgres entre agents** : le port hôte est la seule ressource
   vraiment partagée. Un nom de conteneur distinct ne suffit pas — donner *aussi* un
   port distinct, y compris entre l'agent d'implémentation et son vérificateur sur la
   **même** issue. Cf. *Postgres*.
- **5.** **Fichiers appartenant à `root`** : les conteneurs écrivent en `root` sur les
   volumes montés (`target/`, `node_modules/`). Un `git worktree remove` peut
   alors échouer. Passer `--user "$(id -u):$(id -g)"` au `docker run` quand c'est
   possible ; sinon signaler le `sudo rm -rf` restant, ne pas le lancer soi-même.
- **6.** **`cargo fmt` reformate tout le workspace**, pas seulement les fichiers
   touchés. Sur un dépôt déjà formaté c'est sans effet ; au premier passage,
   vérifier que le diff ne déborde pas de l'issue.
- **7.** **Volume `alcoloco-cargo` appartenant à `root`** : avec `--user "$(id -u):$(id -g)"`,
   `cargo` ne peut pas écrire dans le cache partagé tant que le volume appartient à
   `root`. Rencontré par l'agent de #1, qui l'a corrigé une fois pour toutes par un
   `chown -R 1000:1000` du volume. Si l'erreur réapparaît (volume recréé) :
   ```bash
   docker run --rm -v alcoloco-cargo:/c alpine chown -R 1000:1000 /c
   ```
- **8.** **Gate 5 sans navigateur** : Angular 22 teste avec vitest + jsdom.
   `--browsers=ChromeHeadless` n'est plus une option valide — cf. *Gates*.
- **9.** **`docker compose` est absent de cette machine** — et ce briefing a affirmé le
   contraire jusqu'au 2026-08-19. `docker-compose.yml` n'est donc pas exécutable en
   local : Postgres se lance en `docker run`, cf. *Postgres*.
- **10.** **Les gates Rust ne touchent jamais la base.** Constaté sur #2 : en remplaçant la
   migration par du SQL invalide, `fmt`, `clippy` et `test --workspace` **restent tous
   verts**. Aucun SQL n'est couvert par la CI aujourd'hui. Un agent qui écrit du schéma
   doit donc le sonder lui-même contre un vrai Postgres — les gates verts ne disent
   **rien** sur son SQL. (Dette suivie : cf. *Escalade*, ajout d'un service Postgres à
   la CI.)
- **12.** **Ne jamais laisser un vérificateur hériter du scratchpad de l'auteur.** Rencontré sur #2 :
   l'agent B a trouvé à son arrivée les arbres de build, les mutations et les sondes de l'agent A,
   et a lancé ses trois premiers gates dessus avant de s'en apercevoir. Il les a rejoués sur un
   `git archive` propre — mais c'est exactement le scénario de « confirmation mutuelle » que la
   vérification est censée exclure. **Consigne à donner à tout agent B** : travaille sur un export
   propre de la révision (`git archive <sha>`), jamais sur un arbre que tu n'as pas produit.
- **17.** **Une course concurrente pilotée pas à pas ne teste rien.** Démontré sur #2 : deux
   `psql` avancés à la main, ou un pilote qui attend la réponse d'une session avant de
   faire avancer l'autre, laissent des dizaines de millisecondes entre les deux `COMMIT`
   — largement au-delà de la fenêtre réelle (2–5 ms). Le scénario conclut « ça tient »
   sans jamais avoir ouvert la fenêtre. Pour éprouver une course au `COMMIT`, **synchroniser
   les deux sessions sur une horloge commune** (`pg_sleep` vers un instant absolu), puis
   **balayer le décalage** pour mesurer la largeur de la fenêtre plutôt qu'un point.
- **18.** **`grep -c '^ERROR'` ne compte aucune erreur `psql`** : les messages sont préfixés
   `psql:/tmp/a.sql:1192: `. Utiliser `grep -c 'ERROR:'`. A produit un « 0 erreur » faux
   pendant une passe de l'arbitrage de #2.

- **19.** **Un `cd` vers un worktree survit d'un appel à l'autre.** Commis par l'orchestrateur sur #2 le
   2026-08-20 : un `cd` fait pour un simple `grep` de contrôle a persisté, et deux entrées de journal
   écrites en chemin relatif sont parties dans `.claude/worktrees/<agent>/.claude/issue-log/`. Le
   journal réel a sauté la vérification bloquante qui motivait un arbitrage. **Écrire l'état de
   l'orchestrateur en chemin absolu, toujours**, et vérifier le `git status` d'un worktree avant de
   le supposer propre — un `?? .claude/…` inattendu est la signature de cette erreur.

- **16.** **Un rapport d'agent n'est pas un artefact du dépôt.** Commis par l'orchestrateur sur
   #42 : un chiffre issu du *rapport de fin de tâche* d'un agent a été recopié au journal,
   puis transmis à deux agents suivants comme « le corps de PR affirme… ». Le corps de PR
   ne l'a jamais contenu. Avant d'attribuer une affirmation à un artefact (corps de PR,
   commentaire, fichier), **l'y lire** — `gh api …/pulls/<n> --jq .body`, pas le journal.

- **13.** **`git fetch`/`push` peuvent échouer en `Permission denied (publickey)` alors que
   l'agent SSH tourne.** Constaté sur #41 le 2026-08-20, puis **résolu le même jour** : la clé
   n'était pas chargée dans l'agent, et `ssh` retombait sur une invite de passphrase impossible
   (`ssh_askpass: No such file or directory`). Ce n'était donc **pas** une propriété de
   l'environnement. Diagnostiquer avant de contourner : `ssh-add -l` doit lister la clé de
   `~/.ssh/config` (`IdentityAgent ~/.ssh/agent.sock`), et `ssh -T git@github.com` doit répondre
   `Hi <user>!`. Si oui, SSH marche. Sinon seulement, pousser par HTTPS avec le token `gh`.
- **14.** **`CARGO_TARGET_DIR` hors dépôt évite le problème des fichiers `root`.** Trouvé par
   l'agent de #41 : plutôt que de subir les 855 entrées `root` de `target/`, pointer
   `CARGO_TARGET_DIR` vers le scratchpad. Le worktree reste alors supprimable.
   ⚠ **Monter aussi le chemin visé** (`-v <scratchpad>:<chemin>`) : la ligne `docker run` de
   *Environnement* ne monte que `"$PWD"`, donc un `CARGO_TARGET_DIR` non monté est créé
   **dans** le conteneur et jeté par `--rm` — le worktree reste propre, mais chaque gate
   recompile tout depuis zéro.
- **15.** **Une sonde de trigger peut être interceptée avant le trigger.** Sur #41, la sonde
   de `UPDATE … SET profile_id` construite avec le **même `valid_from`** des deux côtés
   tombe sur `profile_settings_version_unique_start` — elle passe au vert sans avoir
   jamais atteint le trigger qu'elle prétend éprouver. Vérifier *quelle* contrainte
   rejette, pas seulement *qu'il y a* rejet.

- **20.** **Une fonction SQL qui lit une table sans la qualifier est contournable par
   `pg_temp`.** Trouvé sur #41 après quatre passages de review qui l'avaient manquée :
   `pg_temp` est cherché **avant** `public`, donc n'importe quel rôle ayant les seuls droits
   DML — ni `TRUNCATE`, ni propriété, ni superutilisateur — masque la vraie table avec une
   table temporaire et neutralise la fonction. Deux effets mesurés : un trigger de contrainte
   rendu inopérant (profil durablement sans version), et `profile_settings_at()` renvoyant des
   paramètres **fabriqués** dans tout calcul d'alcoolémie. **Toute fonction écrite en base
   qualifie ses relations et ses types non natifs** (`public.profile`, …). Préférer la
   qualification à `SET search_path` sur une fonction SQL : `SET` empêche l'*inlining*
   (mesuré : `Index Scan` → `Function Scan`). Concerne directement #10, #13 et #43.

- **11.** **Un test qui nomme un fichier ne le lit pas forcément.** Deux tests de #2 ont été
   pris en flagrant délit : l'un disait comparer `docker-compose.yml` sans jamais
   l'ouvrir, l'autre disait garantir des UUID v7 en testant la crate `uuid` elle-même.
   La sonde qui tranche est la **mutation** : casse ce que le test prétend garder, et
   exige de le voir rougir.

---

## Escalade — demander un arbitrage utilisateur

- **#37 (modèle d'absorption)** : arbitrage **produit**, il ne peut pas être rendu
  par un agent. Aucun agent ne retire les `#[ignore]` associés.
- **#5 §10.2 et §10.3** (persistance des composantes hors bibliothèque ; pas
  d'échantillonnage et fenêtre des courbes) : points ouverts dans la spec. Le pas
  d'intégration conditionne #16 — si une issue en dépend, demander avant de coder.
- Toute modification de la CI ou de la politique de dépendances.
- **Dette ouverte (2026-08-19, constatée sur #2) : aucun SQL n'est couvert en CI.**
  Les trois gates Rust restent verts avec une migration remplacée par du SQL invalide
  (prouvé, pas déduit). Fermer ce trou = ajouter un service `postgres` au workflow et
  un test d'intégration conditionné à `DATABASE_URL` — donc **une modification de la
  CI**, qui relève de cette section. Mérite sa propre issue ; ne pas la glisser dans
  une PR de fonctionnalité.
- Toute modification d'une migration **déjà mergée** (les migrations sont
  append-only une fois sur `main`).
- Fermeture d'une issue sans PR, ou merge forcé.

**Review facturée** : `/code-review ultra` est déclenchée par l'utilisateur seul —
l'orchestrateur ne peut pas la lancer. La réclamer explicitement, sans chercher à
la contourner, pour : #2 et toute migration, #16 et #17 (moteur de calcul, cœur
de correction du produit), et toute PR touchant aux permissions ou au chiffrement
si la V2+ en introduit.
