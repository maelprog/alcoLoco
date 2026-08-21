# File d'issues — alcoLoco

> **Socle posé, #1 mergée.** PR #38 le 2026-08-17 (`36b3179`) : coefficient Watson, tableau
> §10.1 et arbitrages §10.0-H→K sur `main`. PR #39 le 2026-08-17 (`5a9f96f`) : workspace Cargo,
> app Angular, Postgres local et CI. Corps de #2, #5, #6, #7, #13, #16, #17 et #37 réalignés
> sur GitHub le 2026-08-17. Les trois `TODO` du briefing ouverts par #1 sont soldés
> (2026-08-18) : jobs CI `rust` et `web`, commandes exactes des gates, **pas d'audit de
> sécurité en CI**.

Milestone **V1** : 31 issues (#1–#29, #37 et **#43**). Milestone **V2+** (#30–#36) hors file.
L'ordre est numérique **à une exception près, délibérée** : **#43 est placée avant #7**, qu'elle
bloque, pour que la file reste correcte prise en tête. Ne pas la « remettre dans l'ordre ».
Au-delà de cette exception, l'ordre respecte les « Bloqué par » déclarés dans chaque corps.
Détail d'une issue en cours : `.claude/issue-log/<N>.md`.

- [x] #1  infra  — Bootstrap du dépôt : workspace Rust, app Angular, Postgres local | PR #39 mergée (`5a9f96f`) | journal: issue-log/archive/1.md
- [x] #2  bdd    — Schéma de base initial et outillage de migrations                | PR #42 mergée (`f35a171`) et **PR #41 mergée** le 2026-08-20 (`f525035`, rebase, 7 commits) | journal: issue-log/archive/2.md
- [x] #3  back   — Squelette de l'API axum : config, erreurs, healthcheck, OpenAPI   | PR #45 mergée le 2026-08-20 (squash `56c6f56`) | 3 gates verts (4-5 n/a), 1 aller-retour | journal: issue-log/archive/3.md
- [ ] #4  front  — Squelette de l'app Angular : routing, layout, client HTTP         | dép: #1 | statut: à faire
- [ ] #5  calcul — Arbitrages actés et cas de référence chiffrés                     | statut: à faire
- [ ] #6  back   — Profils : modèle et API CRUD                                      | dép: #3 | statut: à faire
- [ ] #43 bdd    — Fermer le trou de concurrence de l'invariant « un profil a une version » | dép: #2 | bloque #7 | statut: à faire
- [ ] #7  back   — Versionnage de l'historique des paramètres du profil              | dép: #6 **#43** | statut: à faire (⚠ #43 bloquante : invariant faux sous concurrence — noté aussi en commentaire sur l'issue)
- [ ] #8  front  — Écran de sélection de profil                                      | dép: #4 #6 | statut: à faire
- [ ] #9  front  — Écran d'édition du profil et des préférences par défaut           | dép: #8 | statut: à faire
- [ ] #10 bdd    — Bibliothèque de base : modèle et jeu de données initial           | dép: #2 | statut: à faire
- [ ] #11 back   — API bibliothèque : recherche et lecture                           | dép: #3 #10 | statut: à faire
- [ ] #12 front  — Composant de recherche dans la bibliothèque                       | dép: #4 #11 | statut: à faire
- [ ] #13 back   — Modèle de boisson composée et validations                         | dép: #3 #10 | statut: à faire
- [ ] #14 back   — API consommations : création, modification, suppression           | dép: #6 #13 | statut: à faire
- [ ] #15 back   — API historique des consommations d'un profil                      | dép: #14 | statut: à faire
- [ ] #16 calcul — Moteur d'alcoolémie : Widmark, ingestion, absorption, élimination | dép: #5 #13 | statut: à faire
- [ ] #17 calcul — API série temporelle d'alcoolémie (profil et événement)           | dép: #7 #14 #16 | statut: à faire
- [ ] #18 back   — Événements : modèle, CRUD et gestion des participants             | dép: #3 #6 | statut: à faire
- [ ] #19 front  — Saisie libre d'une boisson (mode 1)                               | dép: #4 #14 | statut: à faire
- [ ] #20 front  — Menu mixologie : composition multi-composantes (mode 2)           | dép: #12 #14 #19 | statut: à faire
- [ ] #21 front  — Historique et duplication d'une boisson (mode 3)                  | dép: #15 #19 | statut: à faire
- [ ] #22 front  — Modification et suppression d'une consommation                    | dép: #14 #21 | statut: à faire
- [ ] #23 front  — Écrans événement : création, participants, vue détail             | dép: #4 #18 | statut: à faire
- [ ] #24 front  — Saisie d'une boisson depuis l'écran événement                     | dép: #20 #23 | statut: à faire
- [ ] #25 front  — Affichage de l'alcoolémie instantanée en g/L                      | dép: #17 | statut: à faire
- [ ] #26 front  — Courbes d'alcoolémie comparées des participants                   | dép: #17 #23 | statut: à faire
- [ ] #27 front  — Suivi de son alcoolémie depuis son profil                         | dép: #17 #25 | statut: à faire
- [ ] #28 front  — Avertissement produit : estimation, pas un éthylotest             | dép: #4 | statut: à faire
- [ ] #29 back   — Note de ressenti à la saisie d'une consommation                   | dép: #14 #17 #19 | statut: à faire
- [!] #37 calcul — Arbitrer le modèle d'absorption avant la release V1               | dép: #16 | ARBITRAGE UTILISATEUR — non délégable
