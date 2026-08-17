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
- Historique versionné des paramètres physiologiques du profil (poids, taille, sexe, âge).
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

- **Profile** — identité + paramètres physiologiques courants (poids, taille, sexe, date de
  naissance) et
  préférences de saisie (unité par défaut cL/%, durée d'ingestion par défaut, durée d'absorption
  par défaut).
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
- **[V1]** Le profil porte les paramètres nécessaires au calcul : **poids, taille, sexe et date de
  naissance**, tous obligatoires (l'âge et la taille sont exploités par les équations de Watson,
  §6.2).
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
    antérieure à toute modification trouve toujours une version applicable ;
  - les **préférences de saisie** (unité, durées par défaut) **ne sont pas versionnées** : elles
    n'entrent dans aucun calcul, seulement dans le pré-remplissage du formulaire.
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
par les équations de **Watson (1980)**, qui exploitent poids, taille, âge et sexe — la taille et
l'âge sont donc des données de profil obligatoires.

```
TBW_homme (L) = 2,447 − 0,09156 × âge + 0,1074 × taille(cm) + 0,3362 × poids(kg)
TBW_femme (L) = −2,097            + 0,1069 × taille(cm) + 0,2466 × poids(kg)
```

> **Coefficient d'âge** : `0,09156`, valeur de Watson et al. (1980). Une version antérieure de ce
> document portait `0,09516` — deux chiffres transposés. Corrigé le 2026-08-17 ; ne pas « rétablir ».
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
(`r` = 0,68 homme / 0,55 femme).

### 6.3 Élimination

L'élimination est d'ordre zéro (saturée) :

```
β = 0,15 g/L/h par défaut     (littérature : 0,10 – 0,20)
```

**Décision actée (§10.0-H)** : en V1, `β` est une **constante** du crate `domain`, exposée en
paramètre de fonction pour les tests — ce n'est **pas** une donnée de profil et elle n'apparaît pas
en base. Le réglage par profil relève du calibrage **[V2+]** (#34).

Le résultat est **borné à zéro par le bas** : à alcoolémie nulle, l'élimination s'arrête. Aucune
« dette » ne peut être reportée sur une consommation ultérieure.

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
C'(t) = κ × Σ Rᵢ(t) − β × 1[C(t) > 0]     avec κ = 0,806 / TBW
```

> ⚠ Il est **incorrect** de sommer des courbes individuelles portant chacune son propre `− β × t` :
> on éliminerait alors à `N × β`. Cette règle est une contrainte de correction, pas une option.

**Méthode de calcul.** Le plancher à zéro rend la forme fermée insuffisante (il faut détecter les
instants de retour à zéro). La courbe est donc obtenue par **intégration en avant à pas fixe**, au
pas défini en §10.3. Le procédé reste strictement déterministe et rejouable (§7) : à historique
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
| D | Paramètres profil obligatoires | **Poids, taille, sexe, date de naissance.** |
| E | Note de ressenti par défaut | La suggestion **ne dépasse jamais le niveau 6** : le niveau 7 et au-delà ne sont jamais proposés, l'utilisateur peut les sélectionner lui-même. Barème indicatif : 1 < 0,2 ; 2 : 0,2–0,5 ; 3 : 0,5–0,8 ; 4 : 0,8–1,2 ; 5 : 1,2–1,8 ; 6 : > 1,8 g/L. |
| F | Profil d'absorption | **Trapèze** (§6.4) retenu comme **défaut d'implémentation**. Arbitrage produit final avant release V1 → **#37**. |
| G | Superposition | Les débits `R` se somment ; `β` est global au corps et ne s'applique **qu'une fois** (§6.5). |
| H | Taux d'élimination `β` | **Constante** du crate `domain` en V1 (0,15 g/L/h), exposée en paramètre de fonction pour les tests. Ni donnée de profil, ni colonne en base. Calibrage par profil = **[V2+]** (#34). |
| I | Granularité des courbes | **Pas d'intégration interne fixe à 1 min**, fenêtre = durée de l'événement **+ 3 h** de décroissance. Le `?step=` de l'API (§5.7, #17) ne fait que **sous-échantillonner** la série intégrée, et est **borné à [1 min, 1 h]** — il ne change jamais le pas d'intégration, donc jamais le résultat. |
| J | Effet d'un changement de paramètres profil | La date d'effet (`valid_from`) est **choisie par l'utilisateur**, pour distinguer une correction de saisie d'une évolution réelle. Voir §5.1. |
| K | Âge retenu par Watson | Calculé à l'**heure d'ingestion** de chaque boisson, depuis la date de naissance de la version en vigueur. Aucun âge figé en base (§6.2). |

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
| Widmark pur (durées ignorées) | 0,346 g/L | t = 0 |
| Rampe linéaire sur `t_ing + t_abs` | 0,221 g/L | 50 min |
| **Trapèze — défaut actuel** | **0,227 g/L** | **45,7 min** |
| Absorption exponentielle d'ordre 1 (`k_a = 3 / t_abs`) | 0,255 g/L | 26 min |

Cas de comparaison : homme 80 kg / 180 cm / 30 ans → `TBW` = 45,93 L ; 50 cL à 5 % vol → 19,725 g →
`C₀` = 0,346 g/L ; `t_ing` = 20 min, `t_abs` = 30 min, `β` = 0,15.

> **Tableau recalculé le 2026-08-17.** La ligne « Trapèze » reprenait par erreur les valeurs de la
> ligne « Rampe linéaire ». Les deux modèles ne peuvent pas coïncider : le trapèze concentre
> l'absorption au milieu de la fenêtre et pique donc **plus haut et plus tôt** que la rampe, qui
> l'étale uniformément. Le pic se lit là où `κ × R(τ) = β`, sur la branche descendante de `R` pour
> le trapèze. Les trois autres lignes n'ont bougé que du fait de la correction du coefficient Watson.

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
- **Fenêtre** : durée de l'événement **+ 3 h** de décroissance. Pour un événement à fin ouverte,
  la borne haute est `min(maintenant, fin) + 3 h`.
- **`?step=` de `GET /profiles/{id}/bac` (#17)** : sous-échantillonne la série déjà intégrée,
  **borné à [1 min, 1 h]**, valeur par défaut 1 min. Une valeur hors bornes est rejetée en 400. Ce
  paramètre ne peut pas altérer le résultat du calcul — seulement la densité de points rendus.

Motif du plafond : sans borne, une soirée de 12 h à 10 participants demandée au pas de la seconde
produit ~432 000 points par requête.

### 10.4 Ajout d'un item à la bibliothèque **[V2+]** — ouvert
La spec initiale s'interrompt sur « lors de l'ajout d'un item … (reste à spécifier) ». À reprendre
au moment de la V2.
