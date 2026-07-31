//! Validation against the *real* KCP manifest schema.
//!
//! `schema/knowledge-schema.json` is vendored **verbatim** from the
//! [knowledge-context-protocol](https://github.com/Cantara/knowledge-context-protocol)
//! repo (`schema/knowledge-schema.json`) — see `schema/PROVENANCE.txt` for the exact
//! source revision and checksum. It is embedded at build time so the binary stays a
//! single self-contained artifact with no network dependency, and refreshed by copying
//! the upstream file, never by hand-editing a divergent copy here. Authoring a governed
//! unit that does not pass this schema is a forge defect, so `author-playbook`
//! validates every artifact against it *before* writing (fail-closed).

use boon::{Compiler, Schemas};
use serde_yaml::Mapping;
use std::sync::OnceLock;

/// The verbatim upstream schema, embedded at build time.
const SCHEMA_JSON: &str = include_str!("../schema/knowledge-schema.json");

const SCHEMA_URI: &str = "mem://kcp/knowledge-schema.json";

struct Compiled {
    schemas: Schemas,
    index: boon::SchemaIndex,
}

fn compiled() -> &'static Compiled {
    static CELL: OnceLock<Compiled> = OnceLock::new();
    CELL.get_or_init(|| {
        let doc: serde_json::Value =
            serde_json::from_str(SCHEMA_JSON).expect("vendored KCP schema is valid JSON");
        let mut compiler = Compiler::new();
        compiler
            .add_resource(SCHEMA_URI, doc)
            .expect("vendored KCP schema is a valid JSON Schema resource");
        let mut schemas = Schemas::new();
        let index = compiler
            .compile(SCHEMA_URI, &mut schemas)
            .expect("vendored KCP schema compiles");
        Compiled { schemas, index }
    })
}

/// Validate an assembled manifest against the real KCP schema. `Ok(())` means the
/// artifact conforms; `Err` carries a single human-readable summary of why it did not.
pub fn validate_manifest(manifest: &Mapping) -> Result<(), String> {
    // serde_yaml -> serde_json: our manifests use only string keys and JSON scalars, so
    // this is a faithful, lossless projection into the shape the schema is written over.
    let instance: serde_json::Value =
        serde_json::to_value(manifest).map_err(|e| format!("not representable as JSON: {e}"))?;
    let c = compiled();
    match c.schemas.validate(&instance, c.index) {
        Ok(()) => Ok(()),
        Err(err) => Err(first_lines(&err.to_string())),
    }
}

/// boon renders the full failing sub-tree; keep the first couple of lines so a finding
/// message stays a single legible sentence rather than a wall of nested detail.
fn first_lines(s: &str) -> String {
    let joined = s
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .take(2)
        .collect::<Vec<_>>()
        .join("; ");
    if joined.is_empty() {
        "schema validation failed".to_string()
    } else {
        joined
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(yaml: &str) -> Mapping {
        serde_yaml::from_str(yaml).unwrap()
    }

    #[test]
    fn a_minimal_playbook_manifest_validates() {
        // Proves the vendored schema is loaded and accepts a well-formed kind: playbook.
        let m = map(
            "kcp_version: \"0.30\"\nproject: p\nunits:\n  - id: pb\n    path: playbooks/pb.md\n    kind: playbook\n    intent: run the thing\n    scope: project\n    audience: [agent]\n    load_eligible: true\n    steps:\n      - id: s1\n        uses: a-skill\n",
        );
        assert!(validate_manifest(&m).is_ok(), "{:?}", validate_manifest(&m));
    }

    #[test]
    fn a_playbook_with_action_scope_deny_validates_under_0_32() {
        // RFC-0030 / KCP 0.32: the vendored schema must accept kcp_version "0.32" and a
        // playbook-level action_scope.deny — the normative blanket prohibition.
        let m = map(
            "kcp_version: \"0.32\"\nproject: p\nunits:\n  - id: pb\n    path: playbooks/pb.md\n    kind: playbook\n    intent: run the thing\n    scope: project\n    audience: [agent]\n    load_eligible: true\n    action_scope:\n      deny:\n        tools: [transfer_ownership]\n        paths: [\"legal/hold/**\"]\n    steps:\n      - id: s1\n        uses: a-skill\n",
        );
        assert!(validate_manifest(&m).is_ok(), "{:?}", validate_manifest(&m));
    }

    #[test]
    fn a_manifest_without_project_is_rejected() {
        // Proves the validator is real, not a rubber stamp: `project` is REQUIRED.
        let m = map(
            "kcp_version: \"0.30\"\nunits:\n  - id: u\n    path: p.md\n    intent: i\n    scope: project\n    audience: [agent]\n",
        );
        assert!(validate_manifest(&m).is_err());
    }

    #[test]
    fn a_unit_missing_required_fields_is_rejected() {
        let m = map("project: p\nunits:\n  - id: u\n");
        assert!(validate_manifest(&m).is_err());
    }
}
