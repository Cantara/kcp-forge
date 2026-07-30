# kcp-forge

**Forge loose skill files into governed KCP units — and keep the whole corpus sound.**

A single static binary for corpus-level QA in the
[Knowledge Context Protocol](https://github.com/Cantara/knowledge-context-protocol)
ecosystem. Where `kcp-agent` decides what knowledge to load, kcp-forge keeps the corpus
it plans over honest: parseable, consistently named, cross-referenced, converted, and
**visibly drifting when it drifts**.

```
kcp-forge validate        [--json] [--fix-names] [PATH...]   # corpus structural integrity
kcp-forge convert         [--json] [--apply]     [PATH...]   # skill file → governed KCP unit
kcp-forge drift           [--json]               [PATH...]   # converted vs. source: what moved
kcp-forge author-playbook [--json] [--apply]     [PATH...]   # spec → governed kind: playbook manifest
```

`author-playbook` is `convert`'s counterpart for the `kind: playbook` composition
(SPEC §4.3b). Point it at a spec — any YAML file declaring a `steps:` list — and it
assembles a whole manifest: one `kind: skill` unit per step plus the `kind: playbook`
unit that composes them, carrying the §3.13 authority model (per-step `authority_level`,
a playbook-level ceiling, a multi-source `grant_ceiling`) and each step's §4.3a
`action_scope`. Every artifact is validated against the **real** KCP schema
(`schema/knowledge-schema.json`, vendored verbatim — see `schema/PROVENANCE.txt`) before
it is written: a playbook that would not conform is refused, never emitted.

Exit codes are the contract: **0** clean · **1** findings · **2** tool error.
Don't pipe through anything that swallows them — `kcp-forge validate | tail` exits with
tail's status, not the verdict.

## Why this exists

Agent-augmented development accumulates procedural knowledge as loose files: skill
YAMLs, registers, manifests. The knowledge is real; the corpus rots silently. Measured
on one real corpus of ~680 skills grown over six months:

- **46 files were invalid YAML and nobody knew** — the register generator was
  regex-based, so every broken file still appeared in the index.
- An auto-reflect hook appended notes after the last key; YAML read them as list
  continuations, corrupting `see_also` references along the way.
- A prior skill→KCP conversion (644 units) froze at `kcp_version: 0.7` while the spec
  reached 0.30 — every governed unit in it failing closed — and 34 new skills never
  entered it. Nothing noticed.

kcp-forge replaces the pile of ad-hoc scripts that let all of that hide. On its first
run against that repaired corpus it found one more defect the Python tooling *could
not* see: a duplicate mapping key, which PyYAML resolves silently by discarding data.

## `validate` — fail on universal truths, report diversity

`validate` **fails** only on things true of any corpus: files parse, `name` matches the
filename, `name` and `description` exist, `see_also` targets resolve, files are not
metadata-only shells. Everything else — bespoke body schemas, KCP manifests living
among skills — is a **note**, never a failure.

That split is a hard-won rule. The predecessor gate assumed one skill schema and
flagged 23–53 healthy files across five wrong iterations, because the corpus genuinely
uses four body conventions (`instructions:`, `invoke.prompt:`, `content:`, `prompt:`)
plus bespoke shapes. A gate that cries wolf on healthy files teaches people to ignore
it — measure the corpus before asserting what it needs.

## `convert` + `drift` — the sibling model

`convert` derives a **governed KCP unit** from each skill file and writes it next to
the original:

```
skills/deploy-runbook.yaml          # source of truth — never touched
skills/deploy-runbook.kcp.yaml      # derived: kind: skill, load_eligible, content_hash
```

The sibling carries the governed triple the planner needs — `kind: skill`, an explicit
`load_eligible` grant (without it a governed skill fails closed per RFC-0028), and an
`action_scope` derived only from tools the source itself declares. Its `content_hash`
is a sha256 over the **source**: the drift tether.

`drift` then reports the three rot modes that killed the precedent conversion:

| Finding | Meaning |
|---|---|
| `source-edited` | source changed after conversion; sibling is stale |
| `unconverted` | a skill exists with no sibling (additive drift) |
| `spec-drift` | sibling was generated against an older KCP version |
| `orphaned` | a sibling's source is gone (rename/delete never followed) |

Safety properties, tested:

- `convert` is **dry-run by default**; `--apply` writes.
- A hand-edited sibling is **refused, never clobbered** — each sibling ends in an
  integrity line; if it doesn't verify, your edit survives and the run fails instead.
- An **empty scan is a failure**, not a clean pass. A gate pointed at nothing must say
  so — silence is never evidence.

## CI

```yaml
- run: kcp-forge validate skills          # structure sound?
- run: kcp-forge drift skills             # conversions current?
```

Both read-only, both `--json` capable, both honest in their exit codes.

## Install

```
cargo install --git https://github.com/Cantara/kcp-forge
```

or grab a release binary. No runtime, no dependencies.

## Relationship to the ecosystem

| Tool | Owns |
|---|---|
| [knowledge-context-protocol](https://github.com/Cantara/knowledge-context-protocol) | the spec |
| [kcp-agent](https://github.com/Cantara/kcp-agent) | planning: what loads, and why |
| [kcp-skill](https://github.com/Cantara/kcp-skill) | skill conventions + conformance vectors |
| [kcp-hooks](https://github.com/Cantara/kcp-hooks) | the prompt boundary |
| **kcp-forge** | **the corpus: sound, converted, drift-visible** |

kcp-forge deliberately does **not** re-implement manifest semantics — that is
kcp-agent's domain, and the ecosystem spent v0.28 ("Implementation Parity") closing
exactly that class of duplicate-truth drift. When the `kcp-planner` Rust crate reaches
spec currency, kcp-forge will link it and inherit conformance rather than re-earn it.

## License

Apache-2.0 · Copyright 2026 Cantara
