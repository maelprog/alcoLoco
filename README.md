# alcoLoco

Application de suivi d'alcoolémie. Chaque utilisateur saisit ses consommations — boissons simples
ou cocktails composés — et l'application calcule puis affiche son taux d'alcoolémie dans le temps,
individuellement et en comparaison des autres participants d'un même événement.

> ⚠️ **Avertissement** — L'alcoolémie affichée est une **estimation statistique**. Elle ne peut en
> aucun cas servir à déterminer l'aptitude à conduire.

---

## État du projet

Squelette technique en place : workspace Rust, application Angular, PostgreSQL local et CI. Aucune
fonctionnalité métier n'est encore implémentée — le backend n'expose qu'un point de santé et le
front qu'une page vide.

La source de vérité unique est **[SPEC.md](SPEC.md)**. Toute question de périmètre, de modèle de
domaine ou de règle de calcul s'y tranche ; ce README n'en est qu'un résumé d'entrée.

## Stack technique

| Couche | Techno |
|---|---|
| Backend | Rust — framework **axum** |
| Frontend | TypeScript — **Angular** |
| Base de données | **PostgreSQL** |

---

## Démarrage local

### Prérequis

| Outil | Version de référence | Sert à |
|---|---|---|
| Docker + Docker Compose | v2 (`docker compose`) | base de données locale |
| Rust (`rustup`) | **1.97.1**, avec `rustfmt` et `clippy` | backend |
| Node.js + npm | **22.x** | frontend |

La version de Rust est celle épinglée par la CI (`.github/workflows/ci.yml`) : s'en écarter en local
expose à des écarts de `rustfmt` et de `clippy`.

### Arborescence

```
crates/api/       binaire axum — point d'entrée HTTP
crates/db/        schéma PostgreSQL, migrations et jeu de données de développement
crates/domain/    moteur de calcul, sans I/O ni framework (SPEC.md §7, §9)
web/              application Angular
docker-compose.yml  PostgreSQL local
```

### Base de données

```bash
docker compose up -d          # démarre PostgreSQL 16
docker compose logs -f db     # suit les journaux
docker compose down           # arrête ; ajouter -v pour effacer les données
```

Le service écoute sur `localhost:5432` et les identifiants par défaut sont
`alcoloco` / `alcoloco` / base `alcoloco`. Ils sont surchargeables par variables d'environnement —
utile pour faire tourner plusieurs instances en parallèle :

```bash
POSTGRES_PORT=55432 docker compose -p alcoloco-autre up -d
```

| Variable | Défaut |
|---|---|
| `POSTGRES_USER` | `alcoloco` |
| `POSTGRES_PASSWORD` | `alcoloco` |
| `POSTGRES_DB` | `alcoloco` |
| `POSTGRES_PORT` | `5432` |

### Migrations et jeu de données de développement

Le schéma vit dans `crates/db/migrations/`. Les migrations sont **embarquées à la compilation** et
appliquées par l'outil `db`, qui les enregistre dans la table `_sqlx_migrations` : relancer
`migrate` sur une base déjà à jour ne réexécute rien.

```bash
cargo run -p db -- migrate    # applique les migrations en attente
cargo run -p db -- seed       # insère le jeu de données de développement (base vide)
cargo run -p db -- reset      # supprime le schéma public, remigre, puis seed
```

La chaîne de connexion vient de `DATABASE_URL`, par défaut
`postgres://alcoloco:alcoloco@localhost:5432/alcoloco` — les identifiants du `docker-compose.yml`.

Les migrations sont **append-only** une fois sur `main` : on n'édite jamais un fichier déjà appliqué
(l'outil rejetterait la somme de contrôle), on en ajoute un nouveau.

Points de schéma utiles à connaître avant d'écrire une requête :

- les identifiants sont des `uuid` **sans valeur par défaut** : l'application fournit un **UUID v7**
  (PostgreSQL 16 n'a pas de `uuidv7()` natif) ;
- les paramètres physiologiques (poids, taille, sexe, date de naissance) vivent **uniquement** dans
  `profile_settings_version` ; `profile` ne porte que l'identité et les préférences de saisie, qui
  ne sont pas versionnées ;
- tout profil possède **au moins une version** de paramètres : deux triggers de contrainte différés
  rejettent au `COMMIT` un profil créé sans version, la suppression de sa dernière version et
  l'`UPDATE` qui déplacerait cette dernière version vers un autre profil ; un troisième trigger,
  `profile_keeps_its_id`, rend `profile.id` **immuable** et refuse la renumérotation d'un profil
  **sur-le-champ**. Sans lui, supprimer toutes les versions d'un profil puis renuméroter ce profil
  dans la même transaction validait sans erreur : la clé étrangère est `NO ACTION` sur `UPDATE` et
  ne rejette que **tant qu'une version pointe encore** sur l'ancien identifiant. L'ordre d'écriture
  n'est pas libre pour autant : le profil doit précéder sa première version, la clé étrangère
  `profile_settings_version_profile_id_fkey` n'étant **pas** `DEFERRABLE` — l'ordre inverse est
  refusé **sur-le-champ** (23503), et `SET CONSTRAINTS ALL DEFERRED` n'y change rien ;
- **portée exacte de cette garantie**, et elle n'est pas absolue — `SPEC.md` §5.1 et §10.0-L en sont
  la source de vérité, ce qui suit n'en est que le résumé. Ce qui tient : une transaction
  **s'exécutant seule** qui laisserait un profil existant sans version ne commite pas, pour
  `INSERT`, `UPDATE`, `DELETE`, `COPY` et `MERGE`. Cinq classes y échappent, toutes reproduites
  sur PostgreSQL 16 : **(a)** `TRUNCATE profile_settings_version`, qui ne parcourt aucune ligne et
  ne déclenche donc aucun trigger `FOR EACH ROW` (les profils restent, leurs versions
  disparaissent) ; **(b)** la désactivation ou la suppression des triggers
  (`SET session_replication_role = replica`, `ALTER TABLE … DISABLE TRIGGER USER`, `DROP TRIGGER`) ;
  **(c)** la vérification étant différée, la transaction **qui écrit** elle-même, qui lit son propre
  état intermédiaire entre le `DELETE` et le `COMMIT` — elle ne laisse rien de durable pour autant :
  ou bien le profil est encore à zéro version au `COMMIT` et le `COMMIT` est refusé, ou bien elle a
  remis une version et valide sur un état cohérent ;
  **(d)** la **concurrence** : deux transactions simultanées qui suppriment chacune une *autre* des
  versions d'un même profil valident **toutes deux sans erreur**, chacune voyant subsister la
  version que l'autre retire, et le profil reste durablement à zéro version ;
  **(e)** l'**occultation par `pg_temp`**, **fermée** depuis, et ouverte à tout rôle capable
  d'écrire tant qu'elle ne l'était pas : le `search_path` consulte `pg_temp` avant `public`, et le
  privilège `TEMP` sur une base appartient à `PUBLIC` par défaut, si bien qu'un rôle n'ayant reçu
  que `SELECT`, `INSERT`, `UPDATE` et `DELETE` pouvait créer une table temporaire `profile` vide et
  faire lire *celle-là* à la fonction de trigger, dont le retour anticipé se déclenchait alors à
  chaque appel — les trois triggers devenaient inopérants d'un coup et un profil pouvait perdre
  durablement toutes ses versions ; la même occultation sur `profile_settings_version` injectait un
  poids, une taille, un sexe et une date de naissance fabriqués dans chaque réponse de
  `profile_settings_at()`, donc dans chaque courbe. Ce qui la ferme est la **qualification par le
  schéma** : les corps de fonction écrivent `public.profile` et `public.profile_settings_version`,
  et un nom qualifié ne consulte aucun `search_path`. Qualification plutôt que clause `SET
  search_path`, délibérément : `SET` rendrait `profile_settings_at()` opaque à l'inlining, et elle
  est sur le chemin chaud de chaque requête de courbe. Les classes (a), (b) et (d) laissent un état
  durablement incohérent, où `profile_settings_at()` ne rend aucune ligne pour un profil qui existe
  toujours, sans rien signaler, et (e) en laissait un aussi tant qu'elle était ouverte ;
- **qui atteint quoi** : (a) exige le privilège `TRUNCATE` et (b) le superutilisateur, la propriété
  de la table ou un `GRANT SET ON PARAMETER session_replication_role` (PostgreSQL 15 et suivants)
  — un rôle limité au DML n'atteint ni l'une ni l'autre, et toutes deux
  relèvent d'une restauration, d'une remise à zéro de fixtures ou d'un import en masse. Cela vaut
  pour (a) et (b) **seulement**, et ne doit pas se lire « les droits DML sont sans danger ». **(c),
  (d) et (e), si** : (c) et (d) sont des écritures DML ordinaires, que **n'importe quel** rôle
  capable d'écrire atteint, à commencer par celui de l'application ; (e) n'exigeait même pas cela,
  le privilège `TEMP` que tout rôle détient par défaut suffisait, et c'était la plus large des cinq
  tant qu'elle est restée ouverte — un rôle limité au DML atteignait par elle le trou durable de
  (a) et (b) sans posséder aucun de leurs privilèges, et les **valeurs rendues** par-dessus. Aucune
  révocation ne la ferme en pratique, seule la qualification par le schéma la ferme, et elle est en
  place. (d) est ouverte dans le régime nominal, celui du
  niveau d'isolation par défaut de PostgreSQL comme du pilote employé par le backend ; seules deux
  transactions validant l'une et l'autre en `SERIALIZABLE` sont refusées (`40001`). **Conséquence :
  aucune fonctionnalité — #7, #16 ou une autre — ne peut s'appuyer sur cet invariant en présence
  d'écritures concurrentes tant que l'issue #43 n'est pas fermée** ; #43 porte la fermeture réelle
  et bloque #7 ;
- `profile_settings_at(profile_id, instant)` rend la version en vigueur à un instant donné, avec
  repli sur la plus ancienne (borne basse ouverte). Elle rend **zéro ou une ligne** (`RETURNS SETOF`)
  et **aucune ligne** pour un profil inconnu ou un instant NULL, jamais un repli silencieux. La
  contrepartie du `SETOF` a trois volets : la forme scalaire
  `(profile_settings_at($1, $2)).weight_kg` rend zéro ligne au lieu d'un NULL ; elle est de plus
  **illégale** dans `WHERE`, dans une condition `JOIN … ON`, dans `CASE` et en argument d'agrégat,
  refusée dès l'analyse, qu'une version applicable existe ou non ; et en liste de sélection, un
  résultat vide fait disparaître **la ligne entière** de la requête englobante — `SELECT p.id,
  (profile_settings_at(p.id, NULL)).weight_kg FROM profile p` ne rend aucune ligne pour un profil
  valide qui possède bien une version. D'où la forme à écrire :
  `SELECT * FROM profile_settings_at($1, $2)`, ou `LEFT JOIN LATERAL … ON true` lorsque la ligne de
  la requête englobante doit survivre à un résultat vide ;
- une version ne stocke que sa borne basse : la borne haute est le `valid_from` suivant, ce qui rend
  chevauchements et trous **non représentables** ;
- les durées sont des entiers de secondes (`*_duration_seconds`), les volumes des millilitres
  (`*_ml`).

### Backend

```bash
cargo run -p api              # sert sur http://localhost:8080
curl http://localhost:8080/health   # -> ok
```

L'adresse d'écoute est surchargeable par `ALCOLOCO_API_ADDR` (défaut `0.0.0.0:8080`).

Contrôles qualité, identiques à ceux de la CI :

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --workspace
```

### Frontend

```bash
cd web
npm ci
npm start                     # sert sur http://localhost:4200
npm run build                 # bundle de production dans web/dist/
```

Contrôles qualité, identiques à ceux de la CI :

```bash
npm run lint
npm run test -- --watch=false
```

Les tests unitaires du front tournent sous **Vitest** avec l'environnement `jsdom` : aucun
navigateur n'est nécessaire.

### Intégration continue

Le workflow `.github/workflows/ci.yml` s'exécute sur chaque *pull request* et sur `main`. Il porte
deux jobs : **`rust`** (format, clippy, tests) et **`web`** (lint, tests).

## Périmètre V1

- Profils prédéfinis, **sans authentification** — sélection dans une liste au démarrage.
- Création d'événements et ajout direct de profils participants (pas d'invitation).
- Saisie d'une consommation selon 3 modes : **saisie libre**, **menu mixologie** (boisson composée
  de plusieurs composantes), **duplication depuis l'historique**.
- Bibliothèque de boissons de base fournie par l'application, en lecture seule, avec recherche
  textuelle.
- Calcul d'alcoolémie par la **formule de Widmark**, volume de diffusion estimé par les équations
  de **Watson**.
- Affichage de l'alcoolémie instantanée (g/L de sang) et des courbes des participants en parallèle
  sur la durée de l'événement.
- Historique des consommations par profil, toutes soirées confondues, modifiable et supprimable.
- Historique versionné des paramètres physiologiques du profil (poids, taille, sexe, âge).
- Note de ressenti à la saisie, avec valeur par défaut suggérée selon l'alcoolémie.

### Reporté en V2+

Authentification et comptes · toggle « suivre la consommation » par événement · bibliothèque
personnelle évolutive et bibliothèques des autres participants · export CSV · profil d'alcoolémie
personnalisé avec calibrage · extraction du service mixologie en microservice · intégration à
*Manage Our Home*.

## Modèle de domaine

- **Profile** — identité, paramètres physiologiques courants et préférences de saisie. En base, les
  paramètres physiologiques ne sont pas dupliqués sur `profile` : les « courants » sont ceux de la
  version en vigueur maintenant.
- **ProfileSettingsVersion** — snapshot horodaté des paramètres physiologiques ; chaque consommation
  est rattachée à la version en vigueur à son heure d'ingestion, pour rejouer fidèlement les courbes
  passées.
- **Event** — un événement daté et ses profils participants.
- **Drink** — une boisson consommée : nom, heure et durée d'ingestion, durée d'absorption, note de
  ressenti optionnelle, événement optionnel, une ou plusieurs composantes.
- **DrinkComponent** — nom, degré d'alcool (% vol), quantité en cL ou en % du volume total.
- **LibraryItem** — entrée de la bibliothèque de base : nom, degré par défaut, catégorie
  (alcool / soft).

Relation clé : une consommation appartient **toujours** à l'historique du profil ; le rattachement à
un événement est optionnel.

## Moteur de calcul

Détail complet en [§6 de la spec](SPEC.md#6-règles-de-calcul-de-référence).

- **Alcool ingéré** : `A (g) = volume(mL) × degré(%) / 100 × 0,789`, sommé sur les composantes.
- **Volume de diffusion** : eau corporelle totale (TBW) par **Watson (1980)** à partir du poids, de
  la taille, de l'âge et du sexe ; `C₀ = 0,806 × A / TBW`. Repli sur les constantes de Widmark
  (`r` = 0,68 / 0,55) si une donnée manque. L'âge est celui de l'heure d'ingestion de la boisson.
- **Élimination** : ordre zéro, `β = 0,15 g/L/h` par défaut, **bornée à zéro** — aucune dette
  reportée sur une consommation ultérieure.
- **Profil d'absorption** : **trapèze** (convolution durée d'ingestion × durée d'absorption), retenu
  comme *défaut d'implémentation*. Il dégénère vers la rampe linéaire et vers Widmark pur selon les
  durées saisies.
- **Superposition** : les débits d'apparition se somment, mais l'élimination `β` est globale au
  corps et ne s'applique **qu'une seule fois**. Courbe obtenue par intégration en avant à pas fixe.

Le calcul est **déterministe et rejouable** : à historique identique, courbe identique. Le moteur
est isolé dans un module dédié, testable unitairement.

> 🚧 Le choix du profil d'absorption **n'est pas définitif** : il doit être confronté aux variantes
> (Widmark pur, rampe linéaire, exponentielle d'ordre 1) **avant la release V1** — voir
> [§10.1](SPEC.md#101-modèle-dingestion-et-dabsorption--défaut-retenu-arbitrage-final-avant-release-v1-37)
> et l'issue [#37](https://github.com/maelprog/alcoLoco/issues/37).

## Architecture

La V1 est monolithique, mais découpée pour anticiper trois extractions en microservices : la
**mixologie / bibliothèque**, les **groupes & événements** (délégués à *Manage Our Home*) et le
**moteur d'alcoolémie** (service de calcul pur, sans état). En pratique : modules Rust séparés,
frontières explicites, aucun accès direct aux tables d'un module depuis un autre.

Autres contraintes transverses : horodatages stockés en UTC et affichés dans le fuseau local du
client.

## Points ouverts

| Sujet | Statut |
|---|---|
| Arbitrage final du modèle d'absorption | Bloquant V1 — issue #37 |
| Persistance des composantes hors bibliothèque en V1 | À confirmer ([§10.2](SPEC.md#102-composantes-hors-bibliothèque-en-v1--ouvert)) |
| Pas d'échantillonnage et fenêtre d'affichage des courbes | Acté le 2026-08-17 — 1 min, fenêtre + 3 h ([§10.3](SPEC.md#103-granularité-des-courbes--acté-le-2026-08-17-voir-100-i)) |
| Ajout d'un item à la bibliothèque | Reporté V2+ ([§10.4](SPEC.md#104-ajout-dun-item-à-la-bibliothèque-v2--ouvert)) |
