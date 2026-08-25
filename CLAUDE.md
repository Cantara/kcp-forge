# kcp-forge

Single static Rust binary for corpus-level QA in the [Knowledge Context Protocol](https://github.com/Cantara/knowledge-context-protocol)
ecosystem: `validate`, `convert`, `drift`, `author-playbook` keep a skills corpus
parseable, converted to governed KCP units, and visibly drifting when it drifts.

Read `knowledge.yaml` first — it is the canonical agent-navigable source of truth for
this repo (README, DESIGN.md, and each local skill, with `intent`/`triggers` per unit).

For the shared governed-skill authoring conventions (PROFILE.md, `action_scope` as a
firewall rule) see [kcp-skill](https://github.com/Cantara/kcp-skill) — don't copy its
library content here; this repo only federates to it.

## This repo's local skills (`skills/`)

- `forge-corpus-qa.yaml` — running `validate`/`convert`/`drift` over a corpus in CI.
- `author-kcp-playbook.yaml` — writing a spec and running `author-playbook`.

Both have converted `.kcp.yaml` siblings checked in — re-run `convert --apply` after editing either.

## Gotchas

- `schema/knowledge-schema.json` is vendored **verbatim** (see `schema/PROVENANCE.txt`)
  — never hand-edit; re-vendor from knowledge-context-protocol instead.
- DESIGN.md's command list (`wire`, `stats`, `health`, `clean`) is aspirational — check
  `src/main.rs`'s `Command` enum, not DESIGN.md, for what's actually implemented.
- A hand-edited `.kcp.yaml`/`.playbook.kcp.yaml` sibling is refused, not clobbered —
  edit the source and re-run `convert`/`author-playbook --apply`.
