//! `validate` — corpus structural integrity.
//!
//! Fails only on universal truths: parses as YAML, is a mapping, `name` matches the
//! filename, `name` + `description` present, `see_also` targets resolve, not empty.
//! Everything else — body-schema diversity, manifests living among skills — is a note.
//! The precedent gate assumed one schema and flagged 23–53 healthy files across five
//! wrong iterations before this rule was learned.

use crate::corpus;
use crate::report::Report;
use serde_yaml::Value;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

/// Keys that are metadata rather than substance. A file carrying only these is empty.
const META: &[&str] = &[
    "name",
    "description",
    "version",
    "created",
    "updated",
    "last_updated",
    "tags",
    "see_also",
    "domain",
    "kind",
    "related",
    "related_skills",
    "relatedSkills",
    "trigger_phrases",
    "examples",
    "reflections",
    "priority",
    "status",
    "verified",
    "language",
    "title",
    "source",
];

/// The body conventions the corpus is known to use. Anything else is *diversity*, not
/// a defect.
const BODY_KEYS: &[&str] = &["instructions", "invoke", "content", "prompt"];

fn stem(path: &Path) -> String {
    path.file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string()
}

fn display(path: &Path) -> String {
    path.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string()
}

pub fn is_manifest(doc: &serde_yaml::Mapping) -> bool {
    ["units", "kcp_version", "project"]
        .iter()
        .all(|k| doc.contains_key(Value::from(*k)))
}

pub fn run(paths: &[PathBuf], json: bool, fix_names: bool) -> anyhow::Result<bool> {
    let files = corpus::discover(paths);
    let mut report = Report {
        checked: files.len(),
        ..Default::default()
    };
    if files.is_empty() {
        // A gate pointed at nothing must not report clean — an empty scan is how the
        // precedent tool "validated" a corpus it had silently excluded.
        report.problem(
            "empty-corpus",
            ".",
            "no YAML files found under the given paths",
        );
        report.emit(json);
        return Ok(false);
    }

    // First pass: parse everything, collect the known-name set for reference checks.
    // Names resolve across subdirectories — identity is the basename, wherever it lives.
    let mut parsed: Vec<(PathBuf, serde_yaml::Mapping)> = Vec::new();
    let mut known: BTreeSet<String> = BTreeSet::new();

    for path in &files {
        if corpus::is_sibling(path) {
            // Derived artifacts: only their YAML validity is this command's business.
            if let Ok(raw) = fs::read_to_string(path)
                && serde_yaml::from_str::<Value>(&raw).is_err() {
                    report.problem(
                        "invalid-yaml",
                        &display(path),
                        "derived sibling does not parse",
                    );
                }
            continue;
        }
        let raw = match fs::read_to_string(path) {
            Ok(r) => r,
            Err(e) => {
                report.problem("unreadable", &display(path), e.to_string());
                continue;
            }
        };
        match serde_yaml::from_str::<Value>(&raw) {
            Ok(Value::Mapping(doc)) => {
                known.insert(stem(path));
                parsed.push((path.clone(), doc));
            }
            Ok(other) => {
                report.problem(
                    "not-a-mapping",
                    &display(path),
                    format!("top level is {}", type_name(&other)),
                );
            }
            Err(e) => {
                report.problem("invalid-yaml", &display(path), first_line(&e.to_string()));
            }
        }
    }

    // Second pass: per-file shape checks.
    for (path, doc) in &parsed {
        let file = display(path);
        let file_stem = stem(path);

        if is_manifest(doc) {
            report.note("not-a-skill", &file, "a KCP manifest, not a skill");
            continue;
        }

        for key in ["name", "description"] {
            if doc.get(Value::from(key)).map(is_blank).unwrap_or(true) {
                report.problem("missing-key", &file, format!("no `{key}`"));
            }
        }

        if let Some(Value::String(declared)) = doc.get(Value::from("name"))
            && declared != &file_stem {
                if fix_names && rewrite_name(path, declared, &file_stem)? {
                    report.note("name-fixed", &file, format!("{declared} -> {file_stem}"));
                } else {
                    report.problem(
                        "name-mismatch",
                        &file,
                        format!("declares name={declared:?}, file says {file_stem:?}"),
                    );
                }
            }

        let keys: BTreeSet<&str> = doc.keys().filter_map(|k| k.as_str()).collect();
        let non_meta: Vec<&str> = keys
            .iter()
            .filter(|k| !META.contains(*k))
            .copied()
            .collect();
        if non_meta.is_empty() {
            report.problem("empty", &file, "metadata only, no body of any kind");
        } else if !BODY_KEYS.iter().any(|k| keys.contains(k)) {
            report.note(
                "other-schema",
                &file,
                format!("body under bespoke keys ({})", non_meta.join(", ")),
            );
        }
    }

    // Third pass: cross-references, once the full name set is known.
    for (path, doc) in &parsed {
        if let Some(Value::Sequence(refs)) = doc.get(Value::from("see_also")) {
            for r in refs {
                if let Some(target) = r.as_str()
                    && !known.contains(target) {
                        report.problem(
                            "dangling-ref",
                            &display(path),
                            format!("see_also: {target} (no such skill)"),
                        );
                    }
            }
        }
    }

    report.emit(json);
    Ok(report.clean())
}

fn is_blank(v: &Value) -> bool {
    match v {
        Value::Null => true,
        Value::String(s) => s.trim().is_empty(),
        _ => false,
    }
}

fn type_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Sequence(_) => "sequence",
        Value::Mapping(_) => "mapping",
        Value::Tagged(_) => "tagged",
    }
}

fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or(s).to_string()
}

/// Rewrite `name: <declared>` to `name: <stem>` in place. Returns false when the exact
/// line could not be found — in which case the mismatch stays a problem rather than
/// being silently claimed fixed.
fn rewrite_name(path: &Path, declared: &str, stem: &str) -> anyhow::Result<bool> {
    let raw = fs::read_to_string(path)?;
    let needle = format!("name: {declared}");
    let mut done = false;
    let out: Vec<String> = raw
        .lines()
        .map(|l| {
            if !done && l.trim_end() == needle {
                done = true;
                format!("name: {stem}")
            } else {
                l.to_string()
            }
        })
        .collect();
    if done {
        fs::write(path, out.join("\n") + "\n")?;
    }
    Ok(done)
}
