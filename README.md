# alcoLoco

Application de suivi d'alcoolémie. Chaque utilisateur saisit ses consommations — boissons simples
ou cocktails composés — et l'application calcule puis affiche son taux d'alcoolémie dans le temps,
individuellement et en comparaison des autres participants d'un même événement.

> ⚠️ **Avertissement** — L'alcoolémie affichée est une **estimation statistique**. Elle ne peut en
> aucun cas servir à déterminer l'aptitude à conduire.

---

## État du projet

Phase de spécification : le dépôt ne contient pour l'instant que le document de référence.
Aucun code n'est encore écrit.

La source de vérité unique est **[SPEC.md](SPEC.md)**. Toute question de périmètre, de modèle de
domaine ou de règle de calcul s'y tranche ; ce README n'en est qu'un résumé d'entrée.

## Stack technique

| Couche | Techno |
|---|---|
| Backend | Rust — framework **axum** |
| Frontend | TypeScript — **Angular** |
| Base de données | **PostgreSQL** |

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

- **Profile** — identité, paramètres physiologiques courants et préférences de saisie.
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
