# ADR-0002: Qualitative impact claims only (likely / possible)

- Status: Accepted / Date: 2026-07-02

## Context
Build-time measurement variance (machine load, caches, incremental state, linker, environment) makes per-fix quantitative predictions ("saves 40s") unverifiable. Design review flagged quantitative prescriptions as a credibility trap; fabricated numbers are also the fastest route to the "slop" label in r/rust.

## Decision
Prescriptions state impact as `likely` or `possible`, never fabricated seconds. Measured data (from ADR-0001) is presented as raw before/after observations with environment context, not as predictions. An output lint test asserts no quantitative prediction strings ship (issue #4).

## Consequences
Upside: every claim is defensible; aligns with Core Value 2. Downside: weaker marketing copy; the dogfood log (real measured before/after on agent-witness) carries the persuasion instead.
