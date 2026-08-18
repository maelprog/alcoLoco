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
- `profile_settings_at(profile_id, instant)` rend la version en vigueur à un instant donné, avec
  repli sur la plus ancienne (borne basse ouverte) ;
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
