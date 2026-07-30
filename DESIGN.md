# kcp-forge — Design

**Status:** accepted · 2026-07-30
**Language:** Rust (single static binary, no runtime on the user's machine)
**License:** Apache-2.0

## The problem

Agent-augmented development accumulates *procedural knowledge* as loose files: skill
YAMLs, SKILL.md directories, hand-grown registers, KCP manifests. The knowledge is real;
the corpus around it rots silently.

Measured on one real corpus (2026-07-30, ~680 skills accumulated over six months):

- **46 of 681 files were invalid YAML** and nobody knew. The register generator was
  regex-based, so every broken file still appeared in the index — the register looked
  complete while 7% of the corpus would not parse.
- An auto-reflect hook appended notes at two-space indent after the last key; YAML read
  them as list continuations. The corruption also mangled `see_also` entries into
  multi-word scalars, making healthy cross-references look dangling.
- A prior skill→KCP conversion (644 units, 316 with `action_scope`) sat frozen at
  `kcp_version: 0.7` while the spec reached 0.30.3. It predated eligibility grants, so
  every governed skill in it fails closed today. 34 skills were added after the
  conversion and never entered it. Nothing detected any of this.

Each failure was invisible because the checking lived in ad-hoc Python scripts: no exit
codes anyone trusted, no JSON output, no tests, output piped through summarising tools
that swallowed findings. One script counted 20 problems and never printed them.

kcp-forge replaces that class of script with one tested binary.

## What it is

```
kcp-forge validate  [--json] [--fix-names] [PATH...]   # corpus structural integrity
kcp-forge convert   [--json] [--apply] [PATH...]       # skill file → governed KCP unit
kcp-forge drift     [--json] [PATH...]                 # converted vs. source: what moved
kcp-forge wire      [--json] [--check] [PATH...]       # derive register + aggregate manifest
kcp-forge stats     [--json] [PATH...]                 # corpus statistics
kcp-forge health    [--json] [PATH...]                 # one aggregate verdict for CI
kcp-forge clean     [--json] [--apply] [PATH...]       # repair known rot patterns
```

`validate`, `drift`, `health` are read-only always. `convert` and `clean` are **dry-run by
default**; `--apply` commits. `wire --check` verifies derived artifacts match their
sources without writing — that is the CI mode.

## Design rules (each bought with a specific failure)

1. **`--json` on every subcommand.** Every parsing failure in the precedent came from
   scraping human-formatted output. Machine mode is not an afterthought.
2. **Exit codes are the contract: `0` clean · `1` findings · `2` tool error.** And the
   docs warn against piping through anything that swallows them: `cmd | tail` exits with
   tail's status, which reported a failed commit as success twice in one day.
3. **Fail only on universal truths; report shape diversity.** The precedent corpus had
   *four* body conventions (`instructions`, `invoke.prompt`, `content`, `prompt`) plus
   bespoke shapes. A gate written against one assumed schema flagged 23–53 healthy files
   across five wrong iterations. Universal truths only: parses as YAML, `name` matches
   filename, `name`+`description` present, `see_also` targets resolve, not empty.
   Everything else is a *note*, never a failure.
4. **Destructive operations refuse when unsure.** The precedent repair asserted every
   pre-existing key unchanged before writing, and refused one file rather than corrupt
   it. That refusal preserved data. `clean --apply` does the same or does not ship.
5. **Derived artifacts are never hand-maintained.** Every stale index found this month —
   a series page claiming 9 posts where 50 existed, a published manifest nine versions
   behind its source, the 0.7 conversion — was a copy nobody regenerated. `wire`
   regenerates; `wire --check` fails CI when a derived artifact drifts from source.

## Convert: the sibling model

The novel part. `convert` maps an editor skill file into a **governed KCP unit** and
writes it *next to the original*:

```
~/.claude/skills/deploy-runbook.yaml          # source of truth, untouched
~/.claude/skills/deploy-runbook.kcp.yaml      # derived governed unit
```

The sibling carries:

- `kind: skill`, `intent` (derived from description), `triggers` (from trigger_phrases),
  `audience: [agent]`
- `action_scope.tools` — mapped from declared tools **in the adjudicating runtime's own
  vocabulary** (a `shell.exec` scope in a Claude Code corpus matches nothing; tool
  vocabularies are only meaningful to whoever enforces them)
- `load_eligible: true` — without the explicit grant a governed skill fails closed and
  renders pointer-only (RFC-0028)
- `content_hash` over the **source** file — this is the drift tether
- `x-forge` provenance block: converter version, source path, conversion time

`drift` then has three honest signals, matching the three observed rot modes:

| Signal | Detects | Precedent |
|---|---|---|
| `content_hash` mismatch | source edited after conversion | — |
| source without sibling | additive drift | 34 skills never converted |
| sibling's declared `kcp_version` ≠ current | spec drift | 644 units frozen at 0.7 |

`wire` folds all siblings into one aggregate `knowledge.yaml` for planner navigation.
The siblings are the source of truth for the aggregate; the aggregate is never edited.

## Conformance strategy

kcp-agent ships a Rust planner (`kcp-planner`) with shared conformance vectors. Linking
it is the goal: kcp-forge should **inherit** manifest semantics, not re-earn them — the
ecosystem spent v0.28 ("Implementation Parity") closing exactly this class of drift.

The crate currently sits at 0.19 against a 0.30.3 spec, so v0.1 draws the boundary
conservatively: kcp-forge asserts **structural** truths it can own (YAML validity,
naming, references, hashes) and treats **manifest semantics** as kcp-agent's domain —
`validate` will happily tell you to run `kcp-agent validate` for those. When
`kcp-planner` reaches spec currency, it becomes a dependency (tracked as an issue).

## Non-goals

- Not a linter for skill *content* quality. kcp-skill owns conventions and vectors.
- Not a planner. kcp-agent decides what loads; kcp-forge keeps the corpus it plans over
  sound.
- Not a hook. kcp-hooks fires at the prompt boundary; kcp-forge runs in CI and at the
  command line.

## Verification discipline

TDD: every subcommand's behaviour lands as a failing integration test first, against
fixture corpora that reproduce the real rot patterns above (the appended-tail, the
multi-document file, the quoted-then-prose list item, the stale sibling). Mutation
checks on the gates that matter: a validator whose failure mode has never been observed
is not yet a validator.
