# File d'issues — alcoLoco

Milestone **V1** : 30 issues (#1–#29 + #37). Milestone **V2+** (#30–#36) hors file.
Aucune issue `bug` ouverte — l'ordre est donc numérique, et il se trouve être déjà
topologiquement valide au regard des « Bloqué par » déclarés dans chaque corps.
Détail d'une issue en cours : `.claude/issue-log/<N>.md`.

- [ ] #1  infra  — Bootstrap du dépôt : workspace Rust, app Angular, Postgres local | statut: à faire
- [ ] #2  bdd    — Schéma de base initial et outillage de migrations                | dép: #1 | statut: à faire
- [ ] #3  back   — Squelette de l'API axum : config, erreurs, healthcheck, OpenAPI   | dép: #1 #2 | statut: à faire
- [ ] #4  front  — Squelette de l'app Angular : routing, layout, client HTTP         | dép: #1 | statut: à faire
- [ ] #5  calcul — Arbitrages actés et cas de référence chiffrés                     | statut: à faire
- [ ] #6  back   — Profils : modèle et API CRUD                                      | dép: #3 | statut: à faire
- [ ] #7  back   — Versionnage de l'historique des paramètres du profil              | dép: #6 | statut: à faire
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
