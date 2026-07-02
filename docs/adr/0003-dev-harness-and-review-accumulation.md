# ADR-0003: Development harness and review accumulation

- Status: Accepted / Date: 2026-07-02

## Context
Same rationale as avatar-core ADR-0010 and agent-witness ADR-0003.

## Decision
Issue-driven `/dev`・`/dev-all` harness, self-contained in `.claude/`. Primary gate `just verify`; CI mirrors it (public-OSS trust signal). Review findings promoted to rules (recurs twice → rule; unused 3 months → retire). Deterministic verification core = planted fixture workspaces with known problems (issues #1/#2).

## Consequences
As in agent-witness ADR-0003.
