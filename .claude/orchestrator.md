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
`node`, `npm`, `psql` sont **absents**. Seuls `docker` (29.1.3, démon actif),
`docker compose` (v5.4.0), `git` et `gh` sont disponibles.

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

**Chaque worktree doit poser son propre nom de projet compose *et* son propre port
hôte.** Le nom de projet seul ne suffit pas : le port hôte est publié en dur par la
variable, donc deux projets se disputent quand même `5432`.

```bash
POSTGRES_PORT=54<numéro d'issue sur 2 chiffres> docker compose -p alcoloco-<numéro d'issue> up -d
# ex. issue #2 → POSTGRES_PORT=5402 docker compose -p alcoloco-2 up -d
```

Arrêt et nettoyage en fin d'issue : `docker compose -p alcoloco-<n> down -v`
(le `-v` jette `db_data`, propre au projet).

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

### Arbitrages du 2026-08-17 — appliqués à `SPEC.md`, à ne pas rouvrir

Ces points étaient ouverts ou faux dans la spec initiale. Ils sont désormais tranchés et écrits en
§10.0. Un agent qui croit devoir les rediscuter se trompe : il doit les appliquer.

| Réf. | Décision |
|---|---|
| §6.2 | Coefficient d'âge de Watson = **`0,09516`**, valeur de l'article d'origine (*Am J Clin Nutr* 1980;33:27-39). Une variante `0,09156` circule sur des calculateurs en ligne : **c'est elle qui est fausse**, ne pas l'introduire. |
| §10.0-K | L'**âge** est calculé à l'**heure d'ingestion** de la boisson, depuis la date de naissance de la version en vigueur. Aucun âge n'est stocké. |
| §10.0-H | **`β` est une constante** du crate `domain` (0,15 g/L/h), paramétrable en argument de fonction pour les tests. **Pas de colonne en base, pas de champ d'API.** |
| §10.0-I | Pas d'intégration interne **fixe à 1 min** ; fenêtre = durée de l'événement **+ 3 h**. Le `?step=` de #17 **sous-échantillonne seulement**, borné à **[1 min, 1 h]**, hors bornes → 400. |
| §10.0-J | `valid_from` d'une version de paramètres est **fourni par l'utilisateur** (défaut : maintenant), peut être rétroactif, jamais futur. Versions sans chevauchement ni trou ; la plus ancienne a une borne basse ouverte. Les **préférences de saisie ne sont pas versionnées**. |
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

1. **`rustfmt` / `clippy` absents des images Rust officielles** (`rust:1-bookworm`
   *et* `rust:1-slim-bookworm`). Utiliser `alcoloco-rust:dev`, cf. *Environnement*.
2. **Aucune toolchain hôte.** `cargo`/`node`/`npm`/`psql` sont introuvables et le
   resteront : c'est la configuration normale de cette machine, pas une panne.
3. **`SPEC_CORRECTED.md` n'existe pas** — lire `SPEC.md` (les §§ cités sont bons).
4. **Collision de port Postgres entre worktrees** : `docker compose -p alcoloco-<issue>`
   isole les conteneurs et le volume, **mais pas le port hôte**, publié en dur à
   `${POSTGRES_PORT:-5432}`. Toujours passer *aussi* `POSTGRES_PORT` — cf. *Environnement*.
5. **Fichiers appartenant à `root`** : les conteneurs écrivent en `root` sur les
   volumes montés (`target/`, `node_modules/`). Un `git worktree remove` peut
   alors échouer. Passer `--user "$(id -u):$(id -g)"` au `docker run` quand c'est
   possible ; sinon signaler le `sudo rm -rf` restant, ne pas le lancer soi-même.
6. **`cargo fmt` reformate tout le workspace**, pas seulement les fichiers
   touchés. Sur un dépôt déjà formaté c'est sans effet ; au premier passage,
   vérifier que le diff ne déborde pas de l'issue.
7. **Volume `alcoloco-cargo` appartenant à `root`** : avec `--user "$(id -u):$(id -g)"`,
   `cargo` ne peut pas écrire dans le cache partagé tant que le volume appartient à
   `root`. Rencontré par l'agent de #1, qui l'a corrigé une fois pour toutes par un
   `chown -R 1000:1000` du volume. Si l'erreur réapparaît (volume recréé) :
   ```bash
   docker run --rm -v alcoloco-cargo:/c alpine chown -R 1000:1000 /c
   ```
8. **Gate 5 sans navigateur** : Angular 22 teste avec vitest + jsdom.
   `--browsers=ChromeHeadless` n'est plus une option valide — cf. *Gates*.

---

## Escalade — demander un arbitrage utilisateur

- **#37 (modèle d'absorption)** : arbitrage **produit**, il ne peut pas être rendu
  par un agent. Aucun agent ne retire les `#[ignore]` associés.
- **#5 §10.2 et §10.3** (persistance des composantes hors bibliothèque ; pas
  d'échantillonnage et fenêtre des courbes) : points ouverts dans la spec. Le pas
  d'intégration conditionne #16 — si une issue en dépend, demander avant de coder.
- Toute modification de la CI ou de la politique de dépendances.
- Toute modification d'une migration **déjà mergée** (les migrations sont
  append-only une fois sur `main`).
- Fermeture d'une issue sans PR, ou merge forcé.

**Review facturée** : `/code-review ultra` est déclenchée par l'utilisateur seul —
l'orchestrateur ne peut pas la lancer. La réclamer explicitement, sans chercher à
la contourner, pour : #2 et toute migration, #16 et #17 (moteur de calcul, cœur
de correction du produit), et toute PR touchant aux permissions ou au chiffrement
si la V2+ en introduit.
