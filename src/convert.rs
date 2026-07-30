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
use std::fs;
use std::path::{Path, PathBuf};

/// The spec minor this converter targets. A sibling declaring an older value is stale
/// even when its content hash still matches — the 0.7-freeze failure mode.
pub const TARGET_KCP_VERSION: &str = "0.30";

const INTEGRITY_PREFIX: &str = "# forge-integrity: sha256:";

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
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
    if let Some(Value::Sequence(tools)) = src.doc.get(Value::from("tools")) {
        let mut scope = Mapping::new();
        scope.insert("tools".into(), Value::Sequence(tools.clone()));
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
fn pristine(existing: &str) -> bool {
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
