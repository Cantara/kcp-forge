//! `convert` and `drift` — the sibling model.
//!
//! `convert` derives a governed KCP unit from each skill file and writes it *next to*
//! the original as `<name>.kcp.yaml`. The source stays untouched and stays the source
//! of truth; the sibling is derived, carries a `content_hash` over the source (the
//! drift tether), and ends with an integrity marker so hand edits are detected rather
//! than clobbered.
//!
//! `drift` reports the three rot modes observed on the precedent conversion (644 units
//! frozen at kcp_version 0.7): source edited after conversion, source never converted,
//! and sibling generated against an older spec.

use crate::corpus;
use crate::report::Report;
use crate::validate::is_manifest;
use serde_yaml::{Mapping, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

/// The spec minor this converter targets. A sibling declaring an older value is stale
/// even when its content hash still matches — the 0.7-freeze failure mode.
pub const TARGET_KCP_VERSION: &str = "0.32";

pub const INTEGRITY_PREFIX: &str = "# forge-integrity: sha256:";

pub fn sha256_hex(bytes: &[u8]) -> String {
    // sha2 0.11 dropped LowerHex on the digest output; byte-wise formatting works on
    // both 0.10 and 0.11.
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn display(path: &Path) -> String {
    path.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string()
}

/// A convertible source: a parseable mapping that is a skill (not a manifest, not a
/// derived sibling) and carries at least a name and description to derive from.
struct Source {
    path: PathBuf,
    raw: String,
    doc: Mapping,
}

fn sources(paths: &[PathBuf]) -> Vec<Source> {
    corpus::discover(paths)
        .into_iter()
        .filter(|p| !corpus::is_sibling(p))
        .filter_map(|path| {
            let raw = fs::read_to_string(&path).ok()?;
            let doc: Mapping = match serde_yaml::from_str::<Value>(&raw).ok()? {
                Value::Mapping(m) => m,
                _ => return None,
            };
            if is_manifest(&doc) {
                return None;
            }
            // A file declaring a `steps` list is a playbook spec (§4.3b), authored by
            // `author-playbook` into a governed manifest — never a `kind: skill` source.
            // Without this a spec would be mis-derived into a skill sibling.
            if matches!(doc.get(Value::from("steps")), Some(Value::Sequence(_))) {
                return None;
            }
            let has = |k: &str| {
                doc.get(Value::from(k))
                    .and_then(|v| v.as_str())
                    .map(|s| !s.trim().is_empty())
                    == Some(true)
            };
            if !has("name") || !has("description") {
                return None;
            }
            Some(Source { path, raw, doc })
        })
        .collect()
}

fn sibling_path(source: &Path) -> PathBuf {
    let stem = source.file_stem().unwrap_or_default().to_string_lossy();
    source.with_file_name(format!("{stem}.kcp.yaml"))
}

/// Build the governed unit for a source. Derived, not invented: intent from the
/// description, triggers from trigger_phrases, tools from a declared tools list.
fn governed_unit(src: &Source) -> Mapping {
    let get_str = |k: &str| src.doc.get(Value::from(k)).and_then(|v| v.as_str());
    let stem = src
        .path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();

    let mut unit = Mapping::new();
    unit.insert("id".into(), stem.into());
    unit.insert("path".into(), display(&src.path).into());
    unit.insert("kind".into(), "skill".into());
    unit.insert(
        "intent".into(),
        get_str("description").unwrap_or_default().trim().into(),
    );
    unit.insert("scope".into(), "project".into());
    unit.insert("audience".into(), Value::Sequence(vec!["agent".into()]));

    if let Some(Value::Sequence(phrases)) = src.doc.get(Value::from("trigger_phrases")) {
        unit.insert("triggers".into(), Value::Sequence(phrases.clone()));
    }

    // Without the explicit grant a governed skill fails closed and renders
    // pointer-only (RFC-0028). The grant is the point of converting at all.
    unit.insert("load_eligible".into(), Value::Bool(true));

    // Only declare an action_scope the source itself states. An invented allowlist
    // that is too narrow blocks real work while looking rigorous.
    let mut scope = Mapping::new();
    if let Some(Value::Sequence(tools)) = src.doc.get(Value::from("tools")) {
        scope.insert("tools".into(), Value::Sequence(tools.clone()));
    }
    // RFC-0029 / KCP 0.31: the OPTIONAL negative sibling. Same shape as the allowlist —
    // { tools?, paths?, capabilities? }. A token matching `deny` is refused, and that
    // refusal OVERRIDES any allow (deny-overrides, fail-closed). An absent or empty deny
    // is a no-op, so it is never written. Only what the source itself states, as with
    // the allowlist above.
    if let Some(deny) = deny_scope(&src.doc) {
        scope.insert("deny".into(), Value::Mapping(deny));
    }
    if !scope.is_empty() {
        unit.insert("action_scope".into(), Value::Mapping(scope));
    }

    let mut hash = Mapping::new();
    hash.insert("algorithm".into(), "sha256".into());
    hash.insert("value".into(), sha256_hex(src.raw.as_bytes()).into());
    unit.insert("content_hash".into(), Value::Mapping(hash));

    let mut forge = Mapping::new();
    forge.insert("forge_version".into(), env!("CARGO_PKG_VERSION").into());
    forge.insert("kcp_version".into(), TARGET_KCP_VERSION.into());
    forge.insert("source".into(), display(&src.path).into());
    unit.insert("x-forge".into(), Value::Mapping(forge));

    unit
}

/// The three axes an action_scope bounds (§4.3a): tool names, filesystem paths, named
/// capabilities. `deny` mirrors this shape exactly.
pub const SCOPE_DIMENSIONS: &[&str] = &["tools", "paths", "capabilities"];

/// Read the source's OPTIONAL `deny` declaration into an `action_scope.deny` mapping.
/// RFC-0029: same shape as the allowlist — { tools?, paths?, capabilities? }. Only the
/// dimensions the source actually states are carried; an empty list is dropped. An
/// absent or wholly-empty deny is a no-op and yields `None`, so a no-op sibling is never
/// emitted (matches the spec's "an empty deny object is a no-op").
pub fn deny_scope(doc: &Mapping) -> Option<Mapping> {
    let Some(Value::Mapping(src_deny)) = doc.get(Value::from("deny")) else {
        return None;
    };
    let mut deny = Mapping::new();
    for dim in SCOPE_DIMENSIONS {
        if let Some(Value::Sequence(items)) = src_deny.get(Value::from(*dim))
            && !items.is_empty()
        {
            deny.insert((*dim).into(), Value::Sequence(items.clone()));
        }
    }
    if deny.is_empty() { None } else { Some(deny) }
}

/// The set of string tokens declared for one scope axis; non-strings and absent axes
/// yield the empty set.
pub fn token_set(value: Option<&Value>) -> BTreeSet<String> {
    match value {
        Some(Value::Sequence(items)) => items
            .iter()
            .filter_map(|i| i.as_str().map(String::from))
            .collect(),
        _ => BTreeSet::new(),
    }
}

/// RFC-0029 self-nullifying check. Returns the dimensions where `deny` FULLY CONTAINS
/// the allow set — every allowed token is also refused (deny-overrides), so the skill
/// can touch nothing on that axis and the grant is dead on arrival. An empty allow (there
/// is nothing to nullify) is never a hit. Advisory: a validator SHOULD warn, never fail.
///
/// Containment is over the declared tokens verbatim. That is exact for `tools`/
/// `capabilities`; for `paths` (globs) it catches the identical-glob case an author is
/// most likely to write by mistake without claiming to resolve glob subsumption, which
/// only a runtime enforcer can decide.
fn self_nullifying_dims(scope: &Mapping) -> Vec<&'static str> {
    let Some(Value::Mapping(deny)) = scope.get(Value::from("deny")) else {
        return Vec::new();
    };
    let mut hits = Vec::new();
    for dim in SCOPE_DIMENSIONS {
        let allow = token_set(scope.get(Value::from(*dim)));
        let refused = token_set(deny.get(Value::from(*dim)));
        if !allow.is_empty() && allow.is_subset(&refused) {
            hits.push(*dim);
        }
    }
    hits
}

fn render_sibling(unit: &Mapping) -> String {
    let body = format!(
        "# Derived by kcp-forge from {src}. DO NOT EDIT — edit the source and re-run\n# `kcp-forge convert --apply`. Hand edits are detected and refused, not clobbered.\n{yaml}",
        src = unit
            .get(Value::from("x-forge"))
            .and_then(|f| f.get("source"))
            .and_then(|s| s.as_str())
            .unwrap_or("its source"),
        yaml = serde_yaml::to_string(&Value::Mapping(unit.clone())).expect("unit serializes"),
    );
    format!("{body}{INTEGRITY_PREFIX}{}\n", sha256_hex(body.as_bytes()))
}

/// Is an existing sibling byte-identical to what forge last wrote? The final line is a
/// hash of everything before it; any mismatch — or anything after it — is a hand edit.
pub fn pristine(existing: &str) -> bool {
    let Some(marker_start) = existing.rfind(INTEGRITY_PREFIX) else {
        return false;
    };
    let (body, marker) = existing.split_at(marker_start);
    let recorded = marker
        .trim_end()
        .strip_prefix(INTEGRITY_PREFIX)
        .unwrap_or_default();
    // Anything after the marker line is also a hand edit.
    marker.trim_end().lines().count() == 1 && sha256_hex(body.as_bytes()) == recorded
}

pub fn run_convert(paths: &[PathBuf], json: bool, apply: bool) -> anyhow::Result<bool> {
    let srcs = sources(paths);
    let mut report = Report {
        checked: srcs.len(),
        ..Default::default()
    };

    for src in &srcs {
        let sib = sibling_path(&src.path);
        let unit = governed_unit(src);
        let rendered = render_sibling(&unit);

        // RFC-0029: a deny that fully contains its own allow nullifies the scope. Warn —
        // the sibling is still authored (SHOULD warn, never fail), but the author almost
        // certainly did not intend a procedure permitted to touch nothing.
        if let Some(Value::Mapping(scope)) = unit.get(Value::from("action_scope")) {
            for dim in self_nullifying_dims(scope) {
                report.note(
                    "self-nullifying",
                    &display(&sib),
                    format!("action_scope.deny fully contains allow `{dim}`; the skill can touch no {dim} (deny-overrides)"),
                );
            }
        }

        let existing = fs::read_to_string(&sib).ok();
        match existing {
            Some(ref current) if current == &rendered => {
                report.note("up-to-date", &display(&sib), "sibling current");
            }
            Some(ref current) if !pristine(current) => {
                // Rule 4: refuse when unsure. Someone's hand edit is in that file.
                report.problem(
                    "hand-edited",
                    &display(&sib),
                    "sibling was edited by hand; refusing to overwrite (move your edit into the source, then re-run)",
                );
            }
            _ => {
                if apply {
                    fs::write(&sib, &rendered)?;
                    report.note("written", &display(&sib), "governed sibling written");
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

pub fn run_drift(paths: &[PathBuf], json: bool) -> anyhow::Result<bool> {
    let srcs = sources(paths);
    let all = corpus::discover(paths);
    let mut report = Report {
        checked: srcs.len(),
        ..Default::default()
    };

    for src in &srcs {
        let sib = sibling_path(&src.path);
        let Ok(raw) = fs::read_to_string(&sib) else {
            // Additive drift: 34 skills were added after the precedent conversion ran,
            // and nothing noticed. A source with no sibling is a finding, not an absence.
            report.problem(
                "unconverted",
                &display(&src.path),
                "no governed sibling; run `kcp-forge convert --apply`",
            );
            continue;
        };
        let Ok(Value::Mapping(unit)) = serde_yaml::from_str::<Value>(&raw) else {
            report.problem("invalid-yaml", &display(&sib), "sibling does not parse");
            continue;
        };

        let recorded = unit
            .get(Value::from("content_hash"))
            .and_then(|h| h.get("value"))
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        if recorded != sha256_hex(src.raw.as_bytes()) {
            report.problem(
                "source-edited",
                &display(&src.path),
                "source changed after conversion; sibling is stale",
            );
        }

        let sib_spec = unit
            .get(Value::from("x-forge"))
            .and_then(|f| f.get("kcp_version"))
            .and_then(|v| v.as_str())
            .unwrap_or("(none)");
        if sib_spec != TARGET_KCP_VERSION {
            // The 0.7-freeze mode: hash may match perfectly while the whole conversion
            // predates the spec the planner now enforces.
            report.problem(
                "spec-drift",
                &display(&sib),
                format!("generated for kcp {sib_spec}, converter targets {TARGET_KCP_VERSION}"),
            );
        }
    }

    // Orphans: a sibling whose source is gone is evidence of a rename or deletion the
    // conversion never followed.
    for path in all.iter().filter(|p| corpus::is_sibling(p)) {
        let stem = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let source_name = stem.trim_end_matches(".kcp.yaml");
        let has_source = srcs
            .iter()
            .any(|s| s.path.file_stem().unwrap_or_default().to_string_lossy() == source_name);
        if !has_source {
            report.problem("orphaned", &display(path), "sibling has no source skill");
        }
    }

    report.emit(json);
    Ok(report.clean())
}
