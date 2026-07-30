//! `author-playbook` — assemble a governed `kind: playbook` manifest from a compact spec.
//!
//! The counterpart to `convert`'s `kind: skill` authoring. Where `convert` derives one
//! governed unit from one skill file, `author-playbook` assembles an ordered composition
//! (SPEC §4.3b): a manifest whose steps each enact a `kind: skill` unit, carrying the
//! §3.13 authority model — a per-step `authority_level`, a playbook-level ceiling, and a
//! multi-source `grant_ceiling` — plus the §4.3a `action_scope` each step is bounded by.
//!
//! Two governance properties mirror `convert` exactly:
//!   * **fail-closed** — the assembled artifact is validated against the *real* vendored
//!     KCP schema (`crate::schema`) before it is ever written; a non-conforming artifact
//!     is refused, not emitted;
//!   * **no clobber** — an emitted artifact ends with an integrity marker, and a hand
//!     edit is detected and refused rather than silently overwritten.

use crate::convert::{INTEGRITY_PREFIX, TARGET_KCP_VERSION, pristine, sha256_hex};
use crate::corpus;
use crate::report::Report;
use crate::schema;
use serde_yaml::{Mapping, Sequence, Value};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

/// The fixed, total-ordered §3.13 scale, used when the spec does not declare its own.
const DEFAULT_SCALE: &[&str] = &["observe", "explain", "suggest", "prepare", "commit"];

fn display(path: &Path) -> String {
    path.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string()
}

fn get_str<'a>(m: &'a Mapping, k: &str) -> Option<&'a str> {
    m.get(Value::from(k)).and_then(|v| v.as_str())
}

fn get_seq<'a>(m: &'a Mapping, k: &str) -> Option<&'a Sequence> {
    m.get(Value::from(k)).and_then(|v| v.as_sequence())
}

/// A playbook spec: a parseable, non-sibling mapping declaring a `steps` list. The
/// `steps` key is the discriminator from a `kind: skill` source, matching the exclusion
/// `convert` now applies in the other direction.
struct Spec {
    path: PathBuf,
    raw: String,
    doc: Mapping,
}

fn specs(paths: &[PathBuf]) -> Vec<Spec> {
    corpus::discover(paths)
        .into_iter()
        .filter(|p| !corpus::is_sibling(p))
        .filter_map(|path| {
            let raw = fs::read_to_string(&path).ok()?;
            let doc = match serde_yaml::from_str::<Value>(&raw).ok()? {
                Value::Mapping(m) => m,
                _ => return None,
            };
            matches!(doc.get(Value::from("steps")), Some(Value::Sequence(_))).then_some(Spec {
                path,
                raw,
                doc,
            })
        })
        .collect()
}

/// The manifest sibling is named by the playbook id (the `name` field), not the spec
/// filename — the spec is an input, the id is the authored unit's identity.
fn sibling_path(spec: &Spec, id: &str) -> PathBuf {
    spec.path.with_file_name(format!("{id}.playbook.kcp.yaml"))
}

/// Dedupe while preserving first-seen order.
fn push_unique(into: &mut Sequence, values: &Sequence) {
    for v in values {
        if !into.contains(v) {
            into.push(v.clone());
        }
    }
}

/// Governance lints run *before* the artifact is built (§3.13, §4.3b, §4.3c). Every
/// finding here fails closed: a spec with any problem is never assembled or written.
/// Returns the ordered list of resolved step mappings on success.
fn lint<'a>(spec: &'a Spec, report: &mut Report) -> Option<Vec<&'a Mapping>> {
    let file = display(&spec.path);
    let mut ok = true;
    let mut problem = |kind: &str, msg: String| {
        report.problem(kind, &file, msg);
    };

    let name = get_str(&spec.doc, "name").map(str::trim).unwrap_or("");
    if name.is_empty() {
        problem("missing-key", "no `name`".into());
        ok = false;
    }
    if get_str(&spec.doc, "description")
        .map(str::trim)
        .unwrap_or("")
        .is_empty()
    {
        problem("missing-key", "no `description`".into());
        ok = false;
    }

    let steps: Vec<&Mapping> = get_seq(&spec.doc, "steps")
        .map(|s| s.iter().filter_map(|v| v.as_mapping()).collect())
        .unwrap_or_default();
    let declared_steps = get_seq(&spec.doc, "steps").map(|s| s.len()).unwrap_or(0);
    if steps.len() != declared_steps {
        problem("bad-step", "every step must be a mapping".into());
        ok = false;
    }
    if steps.is_empty() {
        // §4.3b: a playbook with no steps is a manifest error.
        problem("empty-steps", "a playbook needs at least one step".into());
        return None;
    }

    // Step ids: present and unique (§4.3b).
    let mut ids: BTreeSet<&str> = BTreeSet::new();
    for step in &steps {
        let Some(id) = get_str(step, "id") else {
            problem("bad-step", "a step has no `id`".into());
            ok = false;
            continue;
        };
        if !ids.insert(id) {
            problem("duplicate-step", format!("step id {id:?} is not unique"));
            ok = false;
        }
    }

    for step in &steps {
        let sid = get_str(step, "id").unwrap_or("(unnamed)");
        let has_uses = get_str(step, "uses").is_some();
        let has_action = get_str(step, "action").is_some();
        if !has_uses && !has_action {
            // §4.3b: either uses or action MUST be present.
            problem(
                "bad-step",
                format!("step {sid:?} declares neither `uses` nor `action`"),
            );
            ok = false;
        }
        if has_action && !has_uses {
            // §4.3c: an eligible playbook MUST error on an inline (action) step — nothing
            // bounds what it may touch. Every authored playbook is load_eligible.
            problem(
                "inline-step",
                format!(
                    "step {sid:?} is inline (`action`); an eligible playbook admits only `uses` steps"
                ),
            );
            ok = false;
        }
        // §4.3c: an eligible `uses` skill MUST declare an action_scope. We synthesise the
        // skill unit's scope from the step, so the step must state what it may touch.
        if has_uses
            && get_seq(step, "tools").is_none()
            && get_seq(step, "paths").is_none()
            && get_seq(step, "capabilities").is_none()
        {
            problem(
                "step-scope",
                format!(
                    "step {sid:?} names a skill but declares no tools/paths/capabilities to bound it"
                ),
            );
            ok = false;
        }
    }

    // depends_on: references known steps, and the graph is acyclic (§4.3b).
    for step in &steps {
        if let Some(deps) = get_seq(step, "depends_on") {
            for d in deps.iter().filter_map(|v| v.as_str()) {
                if !ids.contains(d) {
                    problem(
                        "unknown-step",
                        format!("depends_on references unknown step {d:?}"),
                    );
                    ok = false;
                }
            }
        }
    }
    if has_cycle(&steps) {
        problem("cyclic-steps", "the depends_on graph has a cycle".into());
        ok = false;
    }

    // grant_ceiling: mandatory_sources MUST all appear (§3.13), and each *_ref MUST
    // resolve — an unresolved reference is a silently-dropped ceiling.
    if let Some(gc) = spec
        .doc
        .get(Value::from("grant_ceiling"))
        .and_then(|v| v.as_mapping())
    {
        let source_ids: BTreeSet<&str> = get_seq(gc, "sources")
            .map(|s| {
                s.iter()
                    .filter_map(|v| v.as_mapping())
                    .filter_map(|m| get_str(m, "id"))
                    .collect()
            })
            .unwrap_or_default();
        if let Some(mandatory) = get_seq(gc, "mandatory_sources") {
            for req in mandatory.iter().filter_map(|v| v.as_str()) {
                if !source_ids.contains(req) {
                    problem(
                        "mandatory-source",
                        format!("grant_ceiling omits mandatory source {req:?}"),
                    );
                    ok = false;
                }
            }
        }
        let agent_ids: BTreeSet<&str> = get_seq(&spec.doc, "agents")
            .map(|s| {
                s.iter()
                    .filter_map(|v| v.as_mapping())
                    .filter_map(|m| get_str(m, "id"))
                    .collect()
            })
            .unwrap_or_default();
        let task_ids: BTreeSet<&str> = get_seq(&spec.doc, "task_types")
            .map(|s| {
                s.iter()
                    .filter_map(|v| v.as_mapping())
                    .filter_map(|m| get_str(m, "id"))
                    .collect()
            })
            .unwrap_or_default();
        for src in get_seq(gc, "sources")
            .into_iter()
            .flatten()
            .filter_map(|v| v.as_mapping())
        {
            let sid = get_str(src, "id").unwrap_or("(unnamed)");
            if let Some(r) = get_str(src, "agent_ref")
                && !agent_ids.contains(r)
            {
                problem(
                    "dangling-ref",
                    format!("source {sid:?} agent_ref {r:?} is not a declared agent"),
                );
                ok = false;
            }
            if let Some(r) = get_str(src, "task_type_ref")
                && !task_ids.contains(r)
            {
                problem(
                    "dangling-ref",
                    format!("source {sid:?} task_type_ref {r:?} is not a declared task_type"),
                );
                ok = false;
            }
        }
    }

    ok.then_some(steps)
}

/// DFS cycle detection over the depends_on edges. An absent depends_on carries no
/// implicit edge here — it is a validity check on the *declared* graph.
fn has_cycle(steps: &[&Mapping]) -> bool {
    use std::collections::HashMap;
    let index: HashMap<&str, &Mapping> = steps
        .iter()
        .filter_map(|s| get_str(s, "id").map(|id| (id, *s)))
        .collect();
    let mut state: HashMap<&str, u8> = HashMap::new(); // 0=unseen,1=on-stack,2=done
    fn dfs<'a>(
        id: &'a str,
        index: &HashMap<&'a str, &'a Mapping>,
        state: &mut std::collections::HashMap<&'a str, u8>,
    ) -> bool {
        match state.get(id) {
            Some(1) => return true,
            Some(2) => return false,
            _ => {}
        }
        state.insert(id, 1);
        if let Some(step) = index.get(id)
            && let Some(deps) = get_seq(step, "depends_on")
        {
            for d in deps.iter().filter_map(|v| v.as_str()) {
                if index.contains_key(d) && dfs(d, index, state) {
                    return true;
                }
            }
        }
        state.insert(id, 2);
        false
    }
    let ids: Vec<&str> = index.keys().copied().collect();
    ids.iter().any(|id| dfs(id, &index, &mut state))
}

/// Assemble the governed manifest. Assumes `lint` has passed.
fn assemble(spec: &Spec, steps: &[&Mapping]) -> Mapping {
    let name = get_str(&spec.doc, "name")
        .unwrap_or_default()
        .trim()
        .to_string();
    let description = get_str(&spec.doc, "description")
        .unwrap_or_default()
        .trim()
        .to_string();

    let mut manifest = Mapping::new();
    manifest.insert("kcp_version".into(), TARGET_KCP_VERSION.into());
    manifest.insert("project".into(), name.clone().into());

    // §3.13 scale — the spec's own, or the fixed default.
    let scale = spec
        .doc
        .get(Value::from("authority_level_scale"))
        .cloned()
        .unwrap_or_else(|| {
            Value::Sequence(DEFAULT_SCALE.iter().map(|s| Value::from(*s)).collect())
        });
    manifest.insert("authority_level_scale".into(), scale);

    // Passthrough of the declared authority collections (§3.13), verbatim.
    for k in ["agents", "task_types", "grant_ceiling"] {
        if let Some(v) = spec.doc.get(Value::from(k)) {
            manifest.insert(k.into(), v.clone());
        }
    }

    // One kind: skill unit per referenced step, plus the playbook unit last.
    let mut units: Sequence = Sequence::new();
    let mut emitted: BTreeSet<String> = BTreeSet::new();
    let mut union_tools = Sequence::new();
    let mut union_paths = Sequence::new();

    for step in steps {
        let uses = get_str(step, "uses").unwrap_or_default();
        let mut action_scope = Mapping::new();
        if let Some(tools) = get_seq(step, "tools") {
            action_scope.insert("tools".into(), Value::Sequence(tools.clone()));
            push_unique(&mut union_tools, tools);
        }
        if let Some(paths) = get_seq(step, "paths") {
            action_scope.insert("paths".into(), Value::Sequence(paths.clone()));
            push_unique(&mut union_paths, paths);
        }
        if let Some(caps) = get_seq(step, "capabilities") {
            action_scope.insert("capabilities".into(), Value::Sequence(caps.clone()));
        }

        if emitted.insert(uses.to_string()) {
            let mut skill = Mapping::new();
            skill.insert("id".into(), uses.into());
            skill.insert("path".into(), format!("skills/{uses}.md").into());
            skill.insert("kind".into(), "skill".into());
            let intent = get_str(step, "intent")
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| format!("Enact the {uses} skill."));
            skill.insert("intent".into(), intent.into());
            skill.insert("scope".into(), "project".into());
            skill.insert("audience".into(), Value::Sequence(vec!["agent".into()]));
            // §4.3c: an eligible skill fails closed without an action_scope; lint has
            // guaranteed the step declared one.
            skill.insert("load_eligible".into(), Value::Bool(true));
            skill.insert("action_scope".into(), Value::Mapping(action_scope));
            units.push(Value::Mapping(skill));
        }
    }

    // The playbook unit.
    let mut pb = Mapping::new();
    pb.insert("id".into(), name.clone().into());
    pb.insert("path".into(), format!("playbooks/{name}.md").into());
    pb.insert("kind".into(), "playbook".into());
    pb.insert("intent".into(), description.into());
    pb.insert("scope".into(), "project".into());
    pb.insert("audience".into(), Value::Sequence(vec!["agent".into()]));
    pb.insert("load_eligible".into(), Value::Bool(true));
    if let Some(level) = get_str(&spec.doc, "authority_level") {
        pb.insert("authority_level".into(), level.into());
    }
    // The declarative action_scope union — computable because every step uses a unit
    // with a declared scope (§4.3b Scope verifiability).
    let mut pb_scope = Mapping::new();
    if !union_tools.is_empty() {
        pb_scope.insert("tools".into(), Value::Sequence(union_tools));
    }
    if !union_paths.is_empty() {
        pb_scope.insert("paths".into(), Value::Sequence(union_paths));
    }
    if !pb_scope.is_empty() {
        pb.insert("action_scope".into(), Value::Mapping(pb_scope));
    }

    // Steps, carrying the per-step ceiling and dependency edges (§3.13/§4.3b).
    let mut out_steps = Sequence::new();
    for step in steps {
        let mut s = Mapping::new();
        s.insert("id".into(), get_str(step, "id").unwrap_or_default().into());
        s.insert(
            "uses".into(),
            get_str(step, "uses").unwrap_or_default().into(),
        );
        for k in [
            "authority_level",
            "depends_on",
            "success_condition",
            "on_failure",
            "timeout",
            "escalation",
        ] {
            if let Some(v) = step.get(Value::from(k)) {
                s.insert(k.into(), v.clone());
            }
        }
        out_steps.push(Value::Mapping(s));
    }
    pb.insert("steps".into(), Value::Sequence(out_steps));
    units.push(Value::Mapping(pb));

    manifest.insert("units".into(), Value::Sequence(units));

    // Provenance, mirroring convert's x-forge block plus the source drift tether.
    let mut forge = Mapping::new();
    forge.insert("forge_version".into(), env!("CARGO_PKG_VERSION").into());
    forge.insert("kcp_version".into(), TARGET_KCP_VERSION.into());
    forge.insert("kind".into(), "playbook".into());
    forge.insert("source".into(), display(&spec.path).into());
    let mut hash = Mapping::new();
    hash.insert("algorithm".into(), "sha256".into());
    hash.insert("value".into(), sha256_hex(spec.raw.as_bytes()).into());
    forge.insert("content_hash".into(), Value::Mapping(hash));
    manifest.insert("x-forge".into(), Value::Mapping(forge));

    manifest
}

/// Render with the same header + trailing integrity marker convert uses, so hand edits
/// are detected rather than clobbered.
fn render(manifest: &Mapping, source: &str) -> String {
    let body = format!(
        "# Authored by kcp-forge from {source}. DO NOT EDIT — edit the spec and re-run\n# `kcp-forge author-playbook --apply`. Hand edits are detected and refused, not clobbered.\n{yaml}",
        yaml =
            serde_yaml::to_string(&Value::Mapping(manifest.clone())).expect("manifest serializes"),
    );
    format!("{body}{INTEGRITY_PREFIX}{}\n", sha256_hex(body.as_bytes()))
}

pub fn run(paths: &[PathBuf], json: bool, apply: bool) -> anyhow::Result<bool> {
    let specs = specs(paths);
    let mut report = Report {
        checked: specs.len(),
        ..Default::default()
    };
    if specs.is_empty() {
        report.problem(
            "no-spec",
            ".",
            "no playbook spec (a YAML file declaring `steps:`) found",
        );
        report.emit(json);
        return Ok(false);
    }

    for spec in &specs {
        let file = display(&spec.path);
        let Some(steps) = lint(spec, &mut report) else {
            continue;
        };
        let manifest = assemble(spec, &steps);

        // Fail-closed: never emit an artifact that does not pass the real KCP schema.
        if let Err(why) = schema::validate_manifest(&manifest) {
            report.problem(
                "schema-invalid",
                &file,
                format!("assembled playbook fails the KCP schema: {why}"),
            );
            continue;
        }
        report.note(
            "schema-valid",
            &file,
            "assembled playbook validates against the vendored KCP schema",
        );

        let name = get_str(&spec.doc, "name").unwrap_or_default().trim();
        let sib = sibling_path(spec, name);
        let rendered = render(&manifest, &file);
        let existing = fs::read_to_string(&sib).ok();
        match existing {
            Some(ref current) if current == &rendered => {
                report.note("up-to-date", &display(&sib), "playbook current");
            }
            Some(ref current) if !pristine(current) => {
                report.problem(
                    "hand-edited",
                    &display(&sib),
                    "artifact was edited by hand; refusing to overwrite (move your edit into the spec, then re-run)",
                );
            }
            _ => {
                if apply {
                    fs::write(&sib, &rendered)?;
                    report.note("written", &display(&sib), "governed playbook written");
                } else {
                    report.note(
                        "would-write",
                        &display(&sib),
                        format!("would write {}", display(&sib)),
                    );
                }
            }
        }
    }

    report.emit(json);
    Ok(report.clean())
}
