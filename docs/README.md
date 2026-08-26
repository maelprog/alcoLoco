# alcoLoco — Documentation

Entry point for the project documentation. Start here to find out which document
answers your question, then follow the link.

> **Language note.** This index is in English; the specification itself is written in
> French and is kept that way on purpose — it is the historical source of truth and is
> quoted verbatim throughout the codebase.

## Documents

| Document | What it contains | When to read it |
|---|---|---|
| [SPEC.md](SPEC.md) | The full functional and technical specification: vision, stack, V1/V2+ scope, domain model, feature-by-feature behaviour, reference computation rules, cross-cutting constraints, and the log of settled decisions and open questions. | Any question about scope, domain model, or expected behaviour. It is the project's **single source of truth**: when code, an issue or a comment disagrees with it, the specification wins. |

For a short overview of the project and how to build and run it, see the
[root README](../README.md).

## Table of contents — SPEC.md

Scope convention used throughout the specification: **[V1]** marks the first
shippable version, **[V2+]** marks what is deferred.

| § | Section | What you will find |
|---|---|---|
| 1 | [Vision](SPEC.md#1-vision) | What alcoLoco is, and the deferred *Manage Our Home* integration. |
| 2 | [Stack technique](SPEC.md#2-stack-technique) | Rust/axum backend, Angular frontend, PostgreSQL. |
| 3 | [Périmètre](SPEC.md#3-périmètre) | What ships in V1 (§3.1) and what is postponed to V2+ (§3.2). |
| 4 | [Modèle de domaine](SPEC.md#4-modèle-de-domaine) | The entities, their fields and the relations between them. |
| 5 | [Fonctionnalités](SPEC.md#5-fonctionnalités) | Behaviour feature by feature: profiles and their versioned settings, events, the three drink-entry modes, base library, history, BAC computation, charts, export **[V2+]**, feeling notes. |
| 6 | [Règles de calcul de référence](SPEC.md#6-règles-de-calcul-de-référence) | The normative formulas: ingested alcohol, Watson diffusion volume, elimination, the trapezoidal absorption profile, superposition and integration. The reference the `domain` crate is checked against. |
| 7 | [Contraintes transverses](SPEC.md#7-contraintes-transverses) | UTC storage, the deterministic and replayable computation requirement, isolation of the computation engine. |
| 8 | [Avertissement produit](SPEC.md#8-avertissement-produit) | The computed BAC is a statistical estimate and must never be used to judge fitness to drive; an explicit warning is required in V1. |
| 9 | [Trajectoire microservices](SPEC.md#9-trajectoire-microservices-v2) | **[V2+]** The three anticipated extractions and the boundaries V1 must respect to stay compatible with them. |
| 10 | [Décisions actées et points ouverts](SPEC.md#10-décisions-actées-et-points-ouverts) | Settled decisions in the §10.0 table (A–L), then the arbitrations still open: ingestion and absorption model (§10.1), out-of-library components (§10.2), curve granularity (§10.3), adding a library item (§10.4). |

## Conventions

- Source files and internal notes cite the specification textually, as `SPEC.md §N`
  (for example `SPEC.md §6.1` in the domain crate and in the SQL migrations). These are
  prose references to the section number, not file paths — they point at this document
  regardless of where it lives in the tree.
