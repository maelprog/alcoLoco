# Arbitrages et reliquats hors file

- **Reliquat #43 / PR #54** — `note_profile_settings_version_removal` (`position()` + `set_config` par ligne) : O(n²) en profils distincts d'une transaction (DELETE sur 16 000 profils : 12,0 s contre 5,9 ms ; 32 000 : 47 s). Condition d'absorption manquante : changement de conception de l'ordre des verrous, à re-mesurer. → issue à ouvrir.
- **Reliquat #43 / PR #54** — chiffres O(n²) du corps de PR et de la migration : ils ne mesurent que le DELETE, sans le contrôle différé au COMMIT. Transaction entière : 13,0 s contre 0,355 s (16 000 profils), 49,2 s contre 0,90 s (32 000). Le rapport entre les schémas est donc exagéré. Condition manquante : arrivé après le tour d'absorption. À reprendre dans l'issue O(n²).
