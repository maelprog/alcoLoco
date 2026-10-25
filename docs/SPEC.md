# alcoLoco — Spécification

> Document de référence du projet — source de vérité unique.
> Convention de portée : **[V1]** = périmètre de la première version livrable, **[V2+]** = repoussé.
> Les arbitrages sont regroupés en §10 : décisions actées en §10.0, points encore ouverts ensuite.

---

## 1. Vision

alcoLoco est une application de suivi d'alcoolémie. Chaque utilisateur saisit ses consommations
(boissons simples ou cocktails composés) ; l'application calcule et affiche son taux d'alcoolémie
dans le temps, individuellement et en comparaison des autres participants d'un même événement.

**[V2+] Intégration « Manage Our Home »** : rattachement à l'application *Manage Our Home* pour la
gestion des groupes et des fonctionnalités transverses (agenda, etc.), via une architecture
microservices. Hors périmètre V1 — mais le découpage des modules doit rester compatible avec une
extraction ultérieure (voir §9).

---

## 2. Stack technique

| Couche | Techno |
|---|---|
| Backend | Rust — framework **axum** |
| Frontend | TypeScript — **Angular** |
| Base de données | **PostgreSQL** |

---

## 3. Périmètre

### 3.1 Inclus en V1

- Profils prédéfinis, **sans authentification** (sélection dans une liste au démarrage).
- Création d'événements et ajout direct de profils participants.
- Saisie de consommation selon 3 modes : saisie libre, menu mixologie, duplication depuis l'historique.
- Bibliothèque de boissons **de base**, fournie par l'application, en lecture seule.
- Calcul d'alcoolémie par la **formule de Widmark**.
- Affichage de l'alcoolémie instantanée (g/L de sang) et des courbes des participants sur la durée
  de l'événement.
- Historique des consommations par profil (toutes soirées confondues), modifiable et supprimable.
- Historique versionné des paramètres physiologiques du profil (poids, taille, sexe, date de
  naissance).
- Note de ressenti à la saisie d'une consommation, avec valeur par défaut proposée selon l'alcoolémie.

### 3.2 Reporté en V2+

- Authentification, comptes, invitations à un événement.
- Toggle « suivre la consommation » activable par événement.
- Bibliothèque personnelle évolutive + bibliothèques des autres participants.
- Extraction du service mixologie en microservice autonome.
- Export CSV des données de consommation et d'alcoolémie.
- Profil d'alcoolémie personnalisé avec calibrage.
- Intégration Manage Our Home.

---

## 4. Modèle de domaine

Entités principales (nommage indicatif) :

- **Profile** — identité et préférences de saisie (unité par défaut cL/%, durée d'ingestion par
  défaut, durée d'absorption par défaut). Le profil ne porte **aucun paramètre physiologique** :
  poids, taille, sexe et date de naissance sont portés par sa **version en vigueur** et se lisent à
  travers elle (§10.0-L).
- **ProfileSettingsVersion** — snapshot horodaté des paramètres physiologiques d'un profil.
  Toute consommation est rattachée à la version en vigueur à son heure d'ingestion, afin de pouvoir
  rejouer fidèlement les courbes passées même après un changement de poids.
- **Event** — un événement daté, avec une liste de profils participants.
- **Drink** — une boisson consommée par un profil : nom, heure d'ingestion, durée d'ingestion,
  durée d'absorption, note de ressenti optionnelle, éventuel rattachement à un événement, et une ou
  plusieurs composantes.
- **DrinkComponent** — une composante d'une boisson : nom, degré d'alcool (% vol), quantité
  (en cL ou en % du volume total de la boisson).
- **LibraryItem** — une entrée de la bibliothèque de base : nom, degré d'alcool par défaut,
  catégorie (alcool / soft).

Relation clé : une consommation appartient **toujours** à l'historique du profil ; le rattachement
à un événement est optionnel (une boisson peut être saisie hors événement, depuis le profil).

---

## 5. Fonctionnalités

### 5.1 Profils

- **[V1]** Aucune authentification. Au lancement, l'utilisateur choisit un profil parmi une liste
  de profils prédéfinis.
- **[V1]** Les paramètres nécessaires au calcul — **poids, taille, sexe et date de naissance** —
  ne sont pas portés par le profil lui-même mais par sa **version de paramètres en vigueur**, et se
  lisent à travers elle (§10.0-L). Ils restent **tous obligatoires** (§10.0-D). L'obligation prend
  la forme d'un invariant tenu par la base, énoncé par son effet observable : **une transaction
  qui laisserait un profil existant sans aucune version de paramètres est rejetée à sa
  validation**, qu'elle s'exécute seule ou en concurrence d'autres transactions. L'ordre des
  écritures en son sein n'est pas libre : le profil doit précéder sa première version, l'ordre
  inverse étant refusé **sur-le-champ**, une version ne pouvant référencer un profil qui n'existe
  pas encore. Sous concurrence (#43), de deux transactions simultanées qui retirent chacune une
  version d'un même profil, l'une peut être refusée, et sa réponse dépend de son niveau
  d'isolation — le détail est en §10.0-L, qui énumère aussi les limites de l'invariant : rien de ce
  qui précède ne doit être lu comme une garantie absolue. La taille et l'âge sont exploités par
  les équations de Watson (§6.2), l'âge étant dérivé de la date de naissance à l'heure d'ingestion
  (§10.0-K).
- **[V1]** Le profil porte les préférences de saisie par défaut :
  - unité de quantité par défaut pour les composantes (cL ou %) ;
  - durée d'ingestion par défaut d'une boisson ;
  - durée d'absorption par défaut.
- **[V1]** L'historique des paramètres du profil est conservé (versionnage), pour recalculer
  fidèlement les courbes historiques et préparer l'export **[V2+]**.
- **[V1] Date d'effet choisie par l'utilisateur (§10.0-J).** À chaque modification des paramètres
  physiologiques, l'API accepte un `valid_from` explicite, dont la valeur par défaut est l'instant
  de la modification. Ce champ existe pour séparer deux gestes que le produit doit distinguer :
  - *« j'ai grossi »* → `valid_from` = maintenant ; les courbes passées ne bougent pas ;
  - *« je m'étais trompé de poids »* → `valid_from` antérieur ; les courbes de la période concernée
    sont recalculées avec la valeur corrigée.

  Règles associées, à respecter par l'implémentation :
  - les versions d'un profil forment une suite **sans chevauchement ni trou** : poser une version à
    `valid_from = T` ferme la précédente à `T` ; toute version déjà entièrement postérieure à `T`
    est **remplacée**, et la réponse indique combien l'ont été ;
  - `valid_from` ne peut pas être dans le futur ;
  - la version la plus ancienne d'un profil a une borne basse ouverte, de sorte qu'une consommation
    antérieure à toute modification trouve une version applicable dès lors que le profil en possède
    au moins une — ce que l'invariant ci-dessus garantit, dans les limites énoncées en §10.0-L ;
  - les **préférences de saisie** (unité, durées par défaut) **ne sont pas versionnées** : elles
    n'entrent dans aucun calcul, seulement dans le pré-remplissage du formulaire.
- **[V1] Suppression d'un profil.** Elle est définitive (pas de corbeille) et emporte, dans une
  seule transaction, tout ce que le profil possède : ses versions de paramètres, ses
  participations aux événements et ses consommations avec leurs composantes. Un événement dont il
  était le seul participant est supprimé avec lui ; les autres événements restent, sans lui, et ses
  courbes n'y sont plus servies. Elle n'est pas refusée par l'invariant ci-dessus, le profil
  n'existant plus à la validation (§10.0-L).
- **[V2+]** Authentification et gestion de comptes.

### 5.2 Événements

- **[V1]** Création d'un événement, avec ajout direct de profils participants (pas d'invitation).
- **[V1]** Depuis la vue d'un événement : liste des participants, alcoolémie courante de chacun,
  courbes comparées, et accès à la saisie d'une consommation.
- **[V2+]** Fonctionnement par invitation.
- **[V2+]** Toggle « suivre la consommation » activable par événement (en V1, le suivi est
  systématiquement actif).

### 5.3 Saisie d'une consommation

Points d'entrée : **depuis le profil** ou **depuis la vue d'un événement**. Les deux aboutissent au
même formulaire et à la même écriture dans l'historique du profil.

Trois modes de saisie :

#### Mode 1 — Saisie libre
Volume (cL), degré d'alcool (% vol), nom. Une seule composante implicite.

#### Mode 2 — Menu mixologie
Composition d'une boisson à partir de plusieurs composantes.

- **Nom de la boisson** — proposé automatiquement, modifiable, **obligatoire** pour valider :
  - 1 composante → le nom de la composante ;
  - 2 composantes → « composante A – composante B » ;
  - 3+ composantes → les deux composantes majoritaires **en volume**, sans distinction entre alcools
    et softs (un jus d'orange majoritaire apparaît donc dans le nom).
- **Liste des composantes**, chacune avec :
  - sa quantité, exprimée en **cL** ou en **% du volume total de la boisson** ; le choix de l'unité
    se fait **par boisson**, et la valeur par défaut vient des préférences du profil ;
  - son degré d'alcool, pré-rempli à la valeur de la bibliothèque lors de la sélection, modifiable ;
  - une croix en fin de ligne pour supprimer la composante.
- Si l'unité retenue est le **%**, le **volume total de la boisson** est **obligatoire** : sans lui
  l'alcoolémie n'est pas calculable. Une boisson en % sans volume total est invalide.
- **Sélection d'une composante depuis la bibliothèque** :
  - **[V1]** barre de recherche sur la **bibliothèque de base** de l'application ; un clic ajoute la
    composante avec son degré pré-rempli, l'utilisateur saisit la quantité présente dans son verre ;
  - **[V1]** saisie d'une composante **absente** de la bibliothèque : nom, degré, quantité
    (sans persistance en bibliothèque en V1) ;
  - **[V2+]** bibliothèque personnelle évolutive, alimentée automatiquement à chaque nouvelle
    composante saisie ;
  - **[V2+]** bibliothèques des autres participants de la soirée ;
  - **[V2+]** la recherche porte par défaut sur toutes les bibliothèques, avec filtre réglable.

#### Mode 3 — Duplication depuis l'historique
Sélection d'une boisson déjà consommée dans l'historique du profil et duplication. L'historique
proposé couvre **toutes** les boissons du profil, pas seulement celles de l'événement en cours.

#### Champs communs aux trois modes
- **Heure d'ingestion** — par défaut l'heure courante, modifiable.
- **Durée d'ingestion** — valeur par défaut issue du profil, modifiable.
- **Durée d'absorption** — valeur par défaut issue du profil, modifiable.
- **Note de ressenti [V1]** — optionnelle, échelle de 1 à 10 (voir §5.9), avec valeur par défaut
  proposée en fonction de l'alcoolémie calculée à l'heure d'ingestion.

### 5.4 Bibliothèque de base

- **[V1]** Livrée avec l'application, en lecture seule, contenant :
  - des alcools courants avec leur degré (vodka, gin, …) ;
  - des softs courants (jus d'orange, coca, …), à 0° d'alcool.
- **[V1]** Recherche textuelle par nom.

### 5.5 Historique des consommations

- **[V1]** Consultation de l'historique complet d'un profil.
- **[V1]** Modification et suppression d'une consommation ; l'alcoolémie et les courbes sont
  recalculées en conséquence.
- **[V1]** Toute saisie alimente l'historique du profil, y compris les saisies faites depuis un
  événement.

### 5.6 Calcul de l'alcoolémie

- **[V1]** Formule de **Widmark** avec volume de diffusion estimé par les équations de **Watson**
  (poids, taille, âge, sexe) — voir §6.2.
- **[V1]** Le calcul tient compte, par boisson, de l'heure d'ingestion, de la durée d'ingestion et
  de la durée d'absorption. Le modèle reliant ces deux durées à la forme de la courbe est le
  **profil d'absorption trapézoïdal** (§6.4) — retenu comme **défaut d'implémentation**, à
  **arbitrer définitivement avant la release V1** (§10.1, issue #37). Le moteur rend ce profil
  interchangeable derrière une abstraction dédiée.
- **[V1]** Le calcul utilise les paramètres physiologiques **en vigueur à l'heure d'ingestion**
  (voir `ProfileSettingsVersion`).
- **[V2+]** Profil personnalisé avec calibrage sur mesures réelles.

### 5.7 Visualisation

- **[V1]** Alcoolémie courante de chaque participant, en **g/L de sang**, en valeur chiffrée.
- **[V1]** Courbes d'alcoolémie des participants affichées **en parallèle**, sur toute la durée de
  l'événement.
- **[V1]** Suivi de sa propre alcoolémie directement depuis son profil, hors événement.

### 5.8 Export **[V2+]**

Extraction en CSV des données de consommation et d'alcoolémie, sur l'intégralité de la période
depuis la création du compte.

### 5.9 Notes de ressenti **[V1]**

À la saisie d'une nouvelle consommation, possibilité d'attribuer une note de ressenti sur son état
courant. La note est **facultative** ; une valeur par défaut est proposée en fonction de
l'alcoolémie calculée à l'heure d'ingestion. Échelle :

| Niveau | Libellé |
|---|---|
| 1 | Aucun effet / quasi sobre |
| 2 | Joyeux |
| 3 | Pompette |
| 4 | Un peu bourré / attaqué |
| 5 | Bourré |
| 6 | Très bourré / bonne cuite |
| 7 | Vomi |
| 8 | Blackout |
| 9 | Coma éthylique |
| 10 | Mort |

**Barème de suggestion** (indicatif, ajustable) :

| Alcoolémie (g/L) | Note suggérée |
|---|---|
| < 0,2 | 1 |
| 0,2 – 0,5 | 2 |
| 0,5 – 0,8 | 3 |
| 0,8 – 1,2 | 4 |
| 1,2 – 1,8 | 5 |
| > 1,8 | 6 |

> La suggestion est **plafonnée au niveau 6** : le niveau 7 et au-delà ne sont jamais proposés
> automatiquement, quelle que soit l'alcoolémie. L'utilisateur reste libre de les sélectionner.

---

## 6. Règles de calcul de référence

### 6.1 Alcool ingéré

```
A (g) = volume (mL) × degré (% vol) / 100 × 0,789
```

Pour une boisson composée, `A` est la somme des contributions de chaque composante. Une composante
exprimée en % est d'abord convertie en volume via le volume total de la boisson.

### 6.2 Volume de diffusion — Watson

**Décision actée** : le coefficient de diffusion est dérivé de l'eau corporelle totale (TBW) estimée
par les équations de **Watson (1980)**, qui exploitent poids, taille, âge et sexe — la taille et la
**date de naissance** sont donc des paramètres obligatoires de la version en vigueur (§10.0-D,
§10.0-L), l'âge étant dérivé de la date de naissance à l'heure d'ingestion (§10.0-K).

```
TBW_homme (L) = 2,447 − 0,09516 × âge + 0,1074 × taille(cm) + 0,3362 × poids(kg)
TBW_femme (L) = −2,097            + 0,1069 × taille(cm) + 0,2466 × poids(kg)
```

> **Coefficient d'âge — attention aux sources divergentes.** La valeur retenue est **`0,09516`**,
> celle de la publication d'origine : Watson PE, Watson ID, Batt RD, « Total body water volumes for
> adult males and females estimated from simple anthropometric measurements », *Am J Clin Nutr*
> 1980;33(1):27-39, dont l'abrégé donne `TBW = 2.447 − 0.09516 A + 0.1074 height + 0.3362 weight`
> (SD 3,76 ; r² 70,4 %). Concordant : [Wikipedia — Body water](https://en.wikipedia.org/wiki/Body_water),
> [Medscape](https://reference.medscape.com/calculator/344/total-body-water-watson-formula), QxMD.
>
> Une variante `0,09156` — les deux chiffres du milieu transposés — circule sur plusieurs
> calculateurs grand public ([Omnicalculator](https://www.omnicalculator.com/health/body-water),
> Calcipedia, iCalculator). **Ce n'est pas la valeur de l'article** : ne pas l'adopter sur la foi
> d'un de ces sites. L'écart est faible (0,11 L sur ~45,8 L, soit 0,24 %) mais visible au 3ᵉ
> décimal des g/L, donc dans les tests de #5 et #16.
>
> **Âge à retenir** : l'âge est calculé à l'**heure d'ingestion de la boisson**, à partir de la date
> de naissance portée par la `ProfileSettingsVersion` en vigueur. Aucun âge n'est figé en base : le
> calcul reste ainsi rejouable à l'identique sans snapshot supplémentaire.

> À noter : l'équation féminine de Watson n'a pas de terme d'âge. L'âge n'influence donc le calcul
> que pour les profils masculins. C'est une propriété du modèle, pas un oubli — si l'on veut un âge
> actif dans les deux cas, il faut basculer sur les équations de Seidl. La date de naissance est
> collectée dans tous les cas.

Le sang étant composé d'environ 80,6 % d'eau :

```
C₀ (g/L de sang) = 0,806 × A / TBW
```

soit, en notation de Widmark, `C₀ = A / (r × M)` avec `r = TBW / (0,806 × M)`.

Repli : si l'âge ou la taille est indisponible, utiliser les constantes de Widmark historiques
(`r` = 0,68 homme / 0,55 femme). Une boisson ingérée avant la date de naissance n'est pas un âge
indisponible : elle ne compte pas, sa contribution est nulle, sans repli (arbitrage du 2026-10-21).

### 6.3 Élimination

L'élimination est d'ordre zéro (saturée) :

```
β = 0,15 g/L/h par défaut     (littérature : 0,10 – 0,20)
```

**Décision actée (§10.0-H)** : en V1, `β` est une **constante** du crate `domain`, exposée en
paramètre de fonction pour les tests — ce n'est **pas** une donnée de profil et elle n'apparaît pas
en base. Le réglage par profil relève du calibrage **[V2+]** (#34).

Le résultat **affiché** est **borné à zéro par le bas** : `C(t) = max(0, brut(t))`, où `brut` est la
forme fermée de l'épisode de consommation en cours, élimination comptée depuis son début (§6.5).
Le plancher porte sur la valeur affichée, pas sur le calcul : tant que les premières fractions d'une
boisson apparaissent moins vite que `β` n'élimine, `brut` est négatif, la courbe affiche zéro, et cette
élimination reste imputée à l'épisode.

Aucune « dette » ne peut être reportée d'un épisode sur le suivant : un épisode se termine quand la
courbe est à zéro **et** qu'aucune boisson n'est encore en cours d'absorption ; la consommation
suivante ouvre un nouvel épisode, dont l'élimination repart de zéro (arbitrage du 2026-10-21).

En ingestion instantanée on retrouve la forme classique `C(t) = C₀ − β × t`.

### 6.4 Profil d'absorption — trapèze **[défaut V1, à arbitrer avant release — #37]**

Une boisson de `A` grammes, ingérée à partir de `t₀`, de durée d'ingestion `a` et de durée
d'absorption `b`, produit un **débit d'apparition de l'alcool dans le sang** `R(t)` (g/h) obtenu par
convolution de deux créneaux : l'alcool est ingéré linéairement sur `a`, et chaque fraction ingérée
passe dans le sang uniformément sur `b`.

Avec `τ = t − t₀`, `m = min(a, b)` et `M = max(a, b)` :

```
          ⎧ A × τ / (a × b)             0 ≤ τ < m      (montée)
R(τ)   =  ⎨ A / M                       m ≤ τ < M      (plateau)
          ⎪ A × (a + b − τ) / (a × b)   M ≤ τ < a + b  (descente)
          ⎩ 0                           sinon
```

L'aire sous `R` vaut exactement `A` : tout l'alcool finit absorbé, à `t₀ + a + b`.

**Dégénérescences utiles** — le trapèze couvre les modèles plus simples sans code dédié :

| Cas | Forme obtenue |
|---|---|
| `a = 0` ou `b = 0` | créneau rectangulaire → rampe linéaire d'absorption |
| `a = 0` **et** `b = 0` | ingestion instantanée → Widmark pur |
| `a = b` | triangle isocèle |

> **Statut** : ce profil est le **défaut d'implémentation**, pas un arbitrage produit définitif.
> Il doit être confronté aux variantes (Widmark pur, rampe linéaire, absorption exponentielle
> d'ordre 1) sur des courbes réelles **avant la release V1** — voir §10.1 et issue **#37**.
> Le moteur isole ce profil derrière une abstraction remplaçable : en changer ne doit coûter
> qu'une implémentation de trait.

### 6.5 Superposition et intégration

**Règle de superposition.** Les débits d'apparition `R` de toutes les consommations d'un profil se
somment linéairement ; l'élimination `β`, elle, est **globale au corps** et ne s'applique
**qu'une seule fois**, quel que soit le nombre de boissons en cours d'absorption.

```
brut'(t) = Σ κᵢ × Rᵢ(t) − β     au sein d'un épisode, avec κᵢ = 0,806 / TBWᵢ
```

> ⚠ Il est **incorrect** de sommer des courbes individuelles portant chacune son propre `− β × t` :
> on éliminerait alors à `N × β`. Cette règle est une contrainte de correction, pas une option.

**Épisodes de consommation** (arbitrage du 2026-10-21). Les consommations d'un profil se
regroupent en épisodes. Au sein d'un épisode ouvert à `t₀` — l'heure d'ingestion de sa première
boisson —, la courbe est la forme fermée, **sans plancher**, l'élimination comptée depuis `t₀` :

```
brut(t) = Σᵢ κᵢ × absorbéᵢ(t − tᵢ) − β × (t − t₀)        C(t) = max(0, brut(t))
```

où `absorbéᵢ(τ) = ∫₀^τ Rᵢ` est la primitive du profil d'absorption (§6.4) et `κᵢ = 0,806 / TBWᵢ`.
`β` n'y apparaît qu'une fois, comme l'exige la règle de superposition.

L'épisode se termine quand la courbe est à zéro **et** qu'aucune de ses boissons n'est encore en
cours d'absorption, soit à `max(E, t₀ + Σ κᵢ × Aᵢ / β)`, `E` étant la dernière fin d'absorption de
l'épisode. Une boisson ingérée avant cet instant rejoint l'épisode — même si la courbe affiche zéro
à ce moment parce qu'une boisson est encore en cours d'absorption ; une boisson ingérée à cet instant
ou après ouvre un nouvel épisode : la dette d'élimination est remise à zéro.

**Méthode de calcul.** Chaque valeur est la forme fermée de son épisode : elle est exacte à tout
instant, sans pas d'intégration, et le pas d'échantillonnage d'une série ne change jamais le
résultat (§10.0-I). Le procédé reste strictement déterministe et rejouable (§7) : à historique
identique, courbe identique au bit près.

Chaque consommation utilise le `TBW` issu de la version de paramètres profil en vigueur à son heure
d'ingestion (§4, `ProfileSettingsVersion`).

---

## 7. Contraintes transverses

- Tous les horodatages sont stockés en UTC et affichés dans le fuseau local du client.
- Le calcul d'alcoolémie doit être **déterministe et rejouable** : à partir de l'historique des
  consommations et des versions de paramètres profil, on doit pouvoir régénérer exactement les
  mêmes courbes.
- Le moteur de calcul est isolé du reste du backend (module dédié, testable unitairement), pour
  préparer son extraction éventuelle et permettre une couverture de tests sérieuse.

---

## 8. Avertissement produit

L'alcoolémie calculée est une **estimation statistique**. Elle ne peut en aucun cas servir à
déterminer l'aptitude à conduire. Un avertissement explicite doit être affiché dans l'application
**[V1]**.

---

## 9. Trajectoire microservices **[V2+]**

Le découpage V1 doit anticiper trois extractions possibles :

1. **Mixologie / bibliothèque** — service autonome de composition de boissons.
2. **Groupes & événements** — délégué à Manage Our Home.
3. **Moteur d'alcoolémie** — service de calcul pur, sans état.

En pratique pour la V1 : modules Rust séparés, frontières explicites, pas d'accès direct aux tables
d'un module depuis un autre.

---

## 10. Décisions actées et points ouverts

### 10.0 Décisions actées

| # | Sujet | Décision |
|---|---|---|
| A | Quantité en % | Le **volume total de la boisson est obligatoire** dès qu'au moins une composante est exprimée en %. Sans lui, la boisson est invalide. |
| B | Composantes « majoritaires » | Majorité **en volume**, toutes composantes confondues — un soft peut donc figurer dans le nom au même titre qu'un alcool. |
| C | Coefficient de diffusion | **Watson**, exploitant poids, taille, âge et sexe (§6.2). Repli sur les constantes Widmark si une donnée manque. |
| D | Paramètres profil obligatoires | **Poids, taille, sexe, date de naissance.** Portés par la version en vigueur, jamais par `Profile` (§10.0-L). |
| E | Note de ressenti par défaut | La suggestion **ne dépasse jamais le niveau 6** : le niveau 7 et au-delà ne sont jamais proposés, l'utilisateur peut les sélectionner lui-même. Barème indicatif : 1 < 0,2 ; 2 : 0,2–0,5 ; 3 : 0,5–0,8 ; 4 : 0,8–1,2 ; 5 : 1,2–1,8 ; 6 : > 1,8 g/L. |
| F | Profil d'absorption | **Trapèze** (§6.4) retenu comme **défaut d'implémentation**. Arbitrage produit final avant release V1 → **#37**. |
| G | Superposition | Les débits `R` se somment ; `β` est global au corps et ne s'applique **qu'une fois** (§6.5). |
| H | Taux d'élimination `β` | **Constante** du crate `domain` en V1 (0,15 g/L/h), exposée en paramètre de fonction pour les tests. Ni donnée de profil, ni colonne en base. Calibrage par profil = **[V2+]** (#34). |
| I | Granularité des courbes | **Pas d'intégration interne fixe à 1 min**, fenêtre = durée de l'événement **+ 3 h** de décroissance. Le `?step=` de l'API (§5.7, #17) ne fait que **sous-échantillonner** la série intégrée, et est **borné à [1 min, 1 h]** — il ne change jamais le pas d'intégration, donc jamais le résultat. |
| J | Effet d'un changement de paramètres profil | La date d'effet (`valid_from`) est **choisie par l'utilisateur**, pour distinguer une correction de saisie d'une évolution réelle. Voir §5.1. |
| K | Âge retenu par Watson | Calculé à l'**heure d'ingestion** de chaque boisson, depuis la date de naissance de la version en vigueur. Aucun âge figé en base (§6.2). |
| L | Emplacement des paramètres physiologiques | Poids, taille, sexe et date de naissance vivent **uniquement** sur `ProfileSettingsVersion` ; `Profile` ne porte que l'identité et les préférences de saisie, et **aucune copie courante** de ces quatre paramètres. Une telle copie serait une **seconde source de vérité**, libre de diverger de la version en vigueur dès la première modification, et qu'aucun calcul ne lirait puisque §10.0-K et §5.6 imposent déjà la version en vigueur **à l'heure d'ingestion**. Contrepartie assumée : l'obligation de §10.0-D n'est plus tenue par le typage mais par un invariant de la base, que cette spécification énonce par son **effet observable** et non par le mécanisme qui le réalise — celui-ci appartient au schéma (#2, #43) et peut évoluer sans que cette ligne change. L'invariant : **une transaction qui laisserait un profil existant sans aucune version de paramètres est rejetée à sa validation** (`COMMIT`), qu'elle s'exécute seule ou en concurrence d'autres ; il tient sur ses écritures ordinaires — `INSERT`, `UPDATE`, `DELETE`, `COPY`, `MERGE`. L'ordre des écritures en son sein n'est pas libre pour autant : le profil doit précéder sa première version, l'ordre inverse étant refusé **sur-le-champ** et non à la validation, une version ne pouvant référencer un profil qui n'existe pas encore. **Sous concurrence**, de deux transactions simultanées qui retirent chacune une version d'un même profil, l'une peut être refusée, selon son niveau d'isolation : en `READ COMMITTED`, elle attend la validation de l'autre, relit, et n'est refusée — par le contrôle lui-même — que si le profil se retrouverait vide ; en `REPEATABLE READ` et `SERIALIZABLE`, elle ne peut pas relire et reçoit un échec de sérialisation (`40001`), à rejouer, **même s'il reste des versions** au profil. Pour tenir cet ordre, le contrôle n'attend que d'autres contrôles portant sur le même profil, ou la suppression de ce profil : il n'introduit pas d'interblocage (`40P01`) avec les écritures de l'application, sauf s'il est avancé en cours de transaction (`SET CONSTRAINTS … IMMEDIATE`). Une suppression de profil par une requête SQL directe, qui ne verrouille pas d'abord les versions du profil comme le fait l'application, peut en revanche s'interbloquer avec le retrait concurrent de l'une de ces versions, selon l'ordre dans lequel la base déclenche ses cascades — l'ordre d'une base restaurée par `pg_dump` y conduit : PostgreSQL annule alors l'une des deux transactions, sans rien laisser à moitié fait, et elle est à rejouer. **Réserve, à ne pas lire comme une garantie absolue** — quatre classes d'opérations, énoncées en classes et non en liste fermée de chemins, une liste fermée redevenant fausse au chemin suivant : l'invariant ne tient pas (a) sur un vidage en bloc de la table des versions (`TRUNCATE`), (b) sur toute opération qui suspend ou retire les contrôles de la base — y compris la suppression, par `DELETE` ou `TRUNCATE`, des lignes de garde sur lesquelles le contrôle sérialise les retraits concurrents : faite pendant que deux transactions retirent chacune une version d'un même profil, la dernière à valider étant en `REPEATABLE READ`, ou en `SERIALIZABLE` alors que l'autre ne l'est pas, elle laisse les deux valider et le profil à zéro version ; si les deux sont en `SERIALIZABLE`, PostgreSQL refuse l'une d'elles (`40001`), et en `READ COMMITTED` le contrôle refuse la dernière (`23000`) ; il suffit pour cela du droit `DELETE` ou `TRUNCATE` sur cette table, que l'application n'exerce jamais : la ligne de garde d'un profil ne disparaît qu'avec le profil lui-même —, ni (c) à l'intérieur de la transaction écrivante avant sa validation, où son propre état intermédiaire lui est visible. La classe (d), **sous concurrence**, est **fermée depuis #43** : deux transactions simultanées retirant chacune une autre des versions d'un même profil validaient toutes deux et le laissaient durablement à zéro version, dans le régime d'isolation par défaut ; elle reste nommée pour que la taxonomie n'oublie pas l'angle mort qu'elle a été. La **suppression d'un profil** (§5.1, #75) ne relève d'aucune de ces classes : elle retire ses versions avec le profil lui-même, dans la même transaction, et à la validation le profil n'existe plus — l'invariant, qui porte sur les profils existants, ne la refuse donc pas. Les cas (a) et (b) laissent un état durablement incohérent, dans lequel la lecture des paramètres d'un profil existant ne rend **aucune ligne** et ne signale rien. Les classes (a), (b) et (c) ne s'atteignent pas par les écritures ordinaires de l'application : elles concernent une restauration, une remise à zéro de jeu d'essai, un import en masse, ou la transaction écrivante elle-même. Le schéma est l'endroit où la portée exacte de chaque classe, et le mécanisme qui tient l'invariant, sont détaillés. Voir §4 et §5.1. |

### 10.1 Modèle d'ingestion et d'absorption — **défaut retenu, arbitrage final avant release V1 (#37)**

Widmark suppose une ingestion instantanée ; la spec ajoute une durée d'ingestion et une durée
d'absorption dont l'effet sur la courbe n'était pas défini.

**Défaut retenu pour l'implémentation : le profil trapézoïdal (§6.4)** — formalisation exacte de la
piste de départ de la spec initiale (ingestion linéaire, puis absorption uniforme de chaque
fraction, élimination courant dès le début). Il a été préféré parce qu'il est le seul candidat où
`durée d'ingestion` et `durée d'absorption` produisent des effets **distincts** : les deux champs
saisis par l'utilisateur restent signifiants.

**Ce défaut n'est pas un arbitrage produit.** Il doit être confronté aux variantes ci-dessous sur
des courbes réelles, avec le porteur du produit, **avant la release V1**.

| Variante | Pic | Instant du pic |
|---|---|---|
| Widmark pur (durées ignorées) | 0,347 g/L | t = 0 |
| Rampe linéaire sur `t_ing + t_abs` | 0,222 g/L | 50 min |
| **Trapèze — défaut actuel** | **0,227 g/L** | **45,7 min** |
| Absorption exponentielle d'ordre 1 (`k_a = 3 / t_abs`) | 0,256 g/L | 26 min |

Cas de comparaison : homme 80 kg / 180 cm / 30 ans → `TBW` = 45,82 L ; 50 cL à 5 % vol → 19,725 g →
`C₀` = 0,347 g/L ; `t_ing` = 20 min, `t_abs` = 30 min, `β` = 0,15.

> **Ligne « Trapèze » corrigée le 2026-08-17.** Elle reprenait par erreur les valeurs de la ligne
> « Rampe linéaire ». Les deux modèles ne peuvent pas coïncider : le trapèze concentre l'absorption
> au milieu de la fenêtre et pique donc **plus haut et plus tôt** que la rampe, qui l'étale
> uniformément. Le pic se lit là où `κ × R(τ) = β`, sur la branche descendante de `R` pour le
> trapèze. Les trois autres lignes sont inchangées.

> **Cohérence avec §6.3 et §6.5 (arbitrage du 2026-10-21).** Ce tableau calcule chaque variante en
> forme fermée, l'élimination courant dès `t₀`. C'est exactement la règle des épisodes : pour une
> boisson seule, l'épisode s'ouvre à son ingestion et le plancher ne porte que sur la valeur affichée.
> Le moteur retrouve donc ces quatre lignes ; le pic du trapèze, 0,227 g/L à 45,7 min, est vérifié
> à travers lui par un test actif.

> Les quatre variantes **convergent après le pic** : on retombe sur `C₀ − β × t`. L'arbitrage ne
> joue donc que sur la première heure suivant chaque verre — mais c'est précisément la fenêtre qui
> compte pendant une soirée, où les consommations s'enchaînent et se superposent.

**Garde-fous en place** — cette décision ne doit pas être oubliée :

1. **Issue #37**, milestone **V1** : bloque la clôture du milestone tant que l'arbitrage n'est pas
   fait. C'est le garde-fou contraignant.
2. **Test `#[ignore]`** sur les cas de référence arbitrés (#5) : visible à chaque `cargo test`.
   Retirer le `#[ignore]` est l'acte qui clôt la décision.
3. **Doc de module** sur l'abstraction `AbsorptionProfile` dans le crate `domain`.

Conséquence de conception, inchangée : le moteur isole le profil d'absorption derrière une
abstraction remplaçable, pour que changer de modèle ne coûte qu'une implémentation de trait.

### 10.2 Composantes hors bibliothèque en V1 — ouvert
En V1 la bibliothèque personnelle n'existe pas : une composante saisie à la main n'est donc
persistée que dans la boisson elle-même, et n'est réutilisable que par duplication depuis
l'historique. À confirmer.

### 10.3 Granularité des courbes — **acté le 2026-08-17, voir §10.0-I**

- **Pas d'intégration interne : 1 min**, fixe. C'est lui qui détermine le résultat du calcul, et il
  n'est jamais exposé au client.
- **Fenêtre** : du début de l'événement à sa fin **+ 3 h** de décroissance, même si la fin fixée
  est encore à venir. Pour un événement à fin ouverte, la borne haute est `maintenant + 3 h`. Un
  événement pas encore commencé est échantillonné quand même (courbe projetée), ses participants
  ayant pu boire avant : à fin fixée (arbitrage du 2026-10-21) comme à fin ouverte (arbitrage du
  2026-10-22). La série n'est vide que si la borne haute précède le début, soit pour un événement
  à fin ouverte qui commence plus de 3 h après la requête.
- **`?step=` de `GET /profiles/{id}/bac` et `GET /events/{id}/bac` (#17)** : sous-échantillonne la
  série, **borné à [1 min, 1 h]**, valeur par défaut 1 min. Une valeur hors bornes est rejetée en
  400. Ce paramètre ne peut pas altérer le résultat du calcul — seulement la densité de points
  rendus.
- **Plafond par requête** : une réponse porte au plus **10 081 valeurs**, toutes séries confondues
  (une semaine au pas d'une minute). Au-delà, le pas n'est pas refusé : il est élargi au plus petit
  multiple d'une minute qui fait tenir le total, et la réponse indique le pas effectivement utilisé
  (arbitrage du 2026-10-21).
- **Instants** : tronqués à la minute — bornes de la fenêtre, début de chaque boisson, instant de
  la valeur courante (arbitrage du 2026-10-21).

Motif du plafond : sans borne, une soirée de 12 h à 10 participants demandée au pas de la seconde
produit ~432 000 points par requête.

### 10.4 Ajout d'un item à la bibliothèque **[V2+]** — ouvert
La spec initiale s'interrompt sur « lors de l'ajout d'un item … (reste à spécifier) ». À reprendre
au moment de la V2.
