//! `kcp-forge author-playbook` — assemble a governed `kind: playbook` manifest from a
//! compact spec, and refuse to emit one that does not pass the *real* KCP schema.
//!
//! The gap this closes: kcp-forge could author `kind: skill` units (`convert`) but had
//! zero `kind: playbook` support — no way to author the ordered, per-step-governed
//! composition SPEC §4.3b describes, carrying the §3.13 authority model (per-step
//! `authority_level`, a `grant_ceiling` with named sources) and §4.3a `action_scope`.
//! Every artifact authored here is validated against the vendored, verbatim
//! `schema/knowledge-schema.json` from the knowledge-context-protocol repo before it is
//! written — fail-closed, never a hand-rolled divergent schema.

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use tempfile::TempDir;

fn forge() -> Command {
    Command::cargo_bin("kcp-forge").unwrap()
}

fn write(dir: &TempDir, rel: &str, content: &str) {
    let path = dir.path().join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

/// A compact playbook spec: three ordered steps, each enacting a `kind: skill` unit,
/// each with its own §3.13 authority ceiling, plus a multi-source `grant_ceiling`.
const SPEC: &str = "\
name: promote-compliance-record
description: Promote a customer compliance record from evidence-gathering through to a committed status change.
authority_level: prepare
agents:
  - id: lara-compliance
    authority_level: prepare
grant_ceiling:
  sources:
    - id: org-risk-policy
      authority_level: prepare
    - id: agent-capability
      agent_ref: lara-compliance
  mandatory_sources:
    - org-risk-policy
steps:
  - id: gather
    uses: gather-evidence
    intent: Gather the compliance evidence for the record.
    authority_level: explain
    tools: [kcp-read]
  - id: prepare-change
    uses: prepare-status-change
    intent: Prepare the status change for review.
    authority_level: prepare
    depends_on: [gather]
    tools: [git]
    paths: [\"records/**\"]
  - id: commit-change
    uses: commit-status-change
    intent: Commit the approved status change.
    authority_level: commit
    depends_on: [prepare-change]
    tools: [git]
    paths: [\"records/**\"]
";

const SIBLING: &str = "promote-compliance-record.playbook.kcp.yaml";

#[test]
fn author_playbook_is_dry_run_by_default() {
    let dir = TempDir::new().unwrap();
    write(&dir, "promote.playbook.yaml", SPEC);
    forge()
        .arg("author-playbook")
        .arg(dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("would write"));
    assert!(!dir.path().join(SIBLING).exists(), "dry-run must not write");
}

#[test]
fn author_playbook_apply_writes_a_schema_valid_governed_playbook() {
    let dir = TempDir::new().unwrap();
    write(&dir, "promote.playbook.yaml", SPEC);
    forge()
        .arg("author-playbook")
        .arg("--apply")
        .arg(dir.path())
        .assert()
        .success()
        // The artifact was checked against the real KCP schema, not merely emitted.
        .stdout(predicate::str::contains("schema-valid"));

    let sibling = dir.path().join(SIBLING);
    assert!(sibling.exists(), "playbook manifest not written");
    let m: serde_yaml::Value =
        serde_yaml::from_str(&fs::read_to_string(&sibling).unwrap()).unwrap();

    // A whole manifest: the schema requires project + units.
    assert_eq!(m["project"], "promote-compliance-record");
    assert_eq!(m["kcp_version"], "0.32");

    // §3.13 authority model at the root.
    let scale = m["authority_level_scale"].as_sequence().unwrap();
    assert_eq!(scale.len(), 5);
    assert_eq!(scale[0], "observe");
    assert_eq!(scale[4], "commit");
    let sources = m["grant_ceiling"]["sources"].as_sequence().unwrap();
    assert_eq!(
        sources.len(),
        2,
        "grant_ceiling must carry its declared sources"
    );
    assert_eq!(sources[0]["id"], "org-risk-policy");
    assert_eq!(sources[0]["authority_level"], "prepare");
    assert_eq!(sources[1]["agent_ref"], "lara-compliance");
    assert_eq!(
        m["grant_ceiling"]["mandatory_sources"][0],
        "org-risk-policy"
    );

    // Units: three referenced skills + the playbook itself.
    let units = m["units"].as_sequence().unwrap();
    let playbook = units
        .iter()
        .find(|u| u["kind"] == "playbook")
        .expect("a kind: playbook unit");
    assert_eq!(playbook["id"], "promote-compliance-record");
    // §4.3c: the eligibility grant — without it the playbook renders pointer-only.
    assert_eq!(playbook["load_eligible"], true);
    // §4.3b: a non-empty steps list, each step referencing a skill, each with a ceiling.
    let steps = playbook["steps"].as_sequence().unwrap();
    assert_eq!(steps.len(), 3);
    assert_eq!(steps[0]["id"], "gather");
    assert_eq!(steps[0]["uses"], "gather-evidence");
    assert_eq!(steps[0]["authority_level"], "explain");
    assert_eq!(steps[2]["authority_level"], "commit");
    assert_eq!(steps[1]["depends_on"][0], "gather");
    // Playbook-level ceiling and the computable action_scope union.
    assert_eq!(playbook["authority_level"], "prepare");
    let pb_tools = playbook["action_scope"]["tools"].as_sequence().unwrap();
    let tool_set: Vec<&str> = pb_tools.iter().filter_map(|v| v.as_str()).collect();
    assert!(tool_set.contains(&"git"));
    assert!(tool_set.contains(&"kcp-read"));

    // Every `uses` resolves to a declared kind: skill unit that is itself eligible and
    // bounded by an action_scope (§4.3a / §4.3c).
    for uses in [
        "gather-evidence",
        "prepare-status-change",
        "commit-status-change",
    ] {
        let skill = units
            .iter()
            .find(|u| u["id"] == uses)
            .unwrap_or_else(|| panic!("missing skill unit {uses}"));
        assert_eq!(skill["kind"], "skill");
        assert_eq!(skill["load_eligible"], true);
        assert!(
            skill["action_scope"]["tools"].as_sequence().is_some(),
            "eligible skill {uses} must declare an action_scope"
        );
    }

    // Provenance, mirroring the convert sibling.
    assert_eq!(m["x-forge"]["kind"], "playbook");
    assert_eq!(m["x-forge"]["source"], "promote.playbook.yaml");
}

#[test]
fn author_playbook_json_is_machine_readable() {
    let dir = TempDir::new().unwrap();
    write(&dir, "promote.playbook.yaml", SPEC);
    let out = forge()
        .arg("author-playbook")
        .arg("--json")
        .arg(dir.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v: serde_json::Value = serde_json::from_slice(&out).expect("stdout must be valid JSON");
    assert_eq!(v["checked"], 1);
    assert!(
        v["findings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["kind"] == "schema-valid"),
        "a schema-valid note must be recorded"
    );
}

#[test]
fn eligible_playbook_rejects_an_inline_step() {
    // §4.3c: an eligible playbook MUST error on an inline (`action`) step — nothing
    // bounds what it may touch. author-playbook fails closed rather than emit it.
    let dir = TempDir::new().unwrap();
    write(
        &dir,
        "bad.playbook.yaml",
        "name: bad\ndescription: has an inline step\nsteps:\n  - id: s1\n    action: do a thing by hand\n",
    );
    forge()
        .arg("author-playbook")
        .arg("--apply")
        .arg(dir.path())
        .assert()
        .code(1)
        .stdout(predicate::str::contains("inline-step"));
    assert!(
        !dir.path().join("bad.playbook.kcp.yaml").exists(),
        "a rejected playbook must not be written"
    );
}

#[test]
fn duplicate_step_ids_are_rejected() {
    // §4.3b: step ids MUST be unique within the playbook.
    let dir = TempDir::new().unwrap();
    write(
        &dir,
        "dup.playbook.yaml",
        "name: dup\ndescription: two steps share an id\nsteps:\n  - id: s\n    uses: a\n    tools: [git]\n  - id: s\n    uses: b\n    tools: [git]\n",
    );
    forge()
        .arg("author-playbook")
        .arg("--apply")
        .arg(dir.path())
        .assert()
        .code(1)
        .stdout(predicate::str::contains("duplicate-step"));
}

#[test]
fn grant_ceiling_missing_a_mandatory_source_is_rejected() {
    // §3.13: a grant_ceiling missing one of its mandatory_sources is a manifest error —
    // this is the gap where a leaf silently drops an org policy ceiling.
    let dir = TempDir::new().unwrap();
    write(
        &dir,
        "leak.playbook.yaml",
        "name: leak\ndescription: drops the org policy ceiling\ngrant_ceiling:\n  sources:\n    - id: task-ceiling\n      authority_level: suggest\n  mandatory_sources:\n    - org-risk-policy\nsteps:\n  - id: s\n    uses: a\n    tools: [git]\n",
    );
    forge()
        .arg("author-playbook")
        .arg("--apply")
        .arg(dir.path())
        .assert()
        .code(1)
        .stdout(predicate::str::contains("mandatory-source"));
}

// RFC-0030 / KCP 0.32: `action_scope.deny` on a `kind: playbook` unit is a blanket
// prohibition over EVERY step — normative for enactment, unlike the rest of the playbook
// `action_scope` envelope, which stays declarative. The effective denylist for a step is
// the per-dimension UNION of the playbook's deny and the used skill's deny; a match in
// either refuses, overriding any allow (§4.3a deny-first). A downstream KCP consumer
// enforces this at run time; forge's job is to author it faithfully.
const SPEC_WITH_DENY: &str = "\
name: quarantine-cleanup
description: Delete quarantined records without ever touching material under legal hold.
authority_level: commit
deny:
  tools: [transfer_ownership]
  paths: [\"legal/hold/**\"]
steps:
  - id: identify
    uses: find-quarantined
    intent: Identify the quarantined records due for deletion.
    authority_level: observe
    tools: [kcp-read]
  - id: purge
    uses: purge-records
    intent: Delete the identified records.
    authority_level: commit
    depends_on: [identify]
    tools: [rm]
    paths: [\"records/quarantine/**\"]
    deny:
      tools: [shell]
";

#[test]
fn author_playbook_emits_playbook_level_deny() {
    let dir = TempDir::new().unwrap();
    write(&dir, "cleanup.playbook.yaml", SPEC_WITH_DENY);
    forge()
        .arg("author-playbook")
        .arg("--apply")
        .arg(dir.path())
        .assert()
        .success()
        // The deny-carrying artifact still passes the real (v0.32) KCP schema.
        .stdout(predicate::str::contains("schema-valid"));

    let sibling = dir.path().join("quarantine-cleanup.playbook.kcp.yaml");
    let m: serde_yaml::Value =
        serde_yaml::from_str(&fs::read_to_string(&sibling).unwrap()).unwrap();
    let units = m["units"].as_sequence().unwrap();
    let playbook = units
        .iter()
        .find(|u| u["kind"] == "playbook")
        .expect("a kind: playbook unit");

    // RFC-0030: the blanket prohibition sits on the playbook unit itself, in the same
    // { tools?, paths?, capabilities? } shape as the §4.3a skill-level deny.
    assert_eq!(
        playbook["action_scope"]["deny"]["tools"][0],
        "transfer_ownership"
    );
    assert_eq!(
        playbook["action_scope"]["deny"]["paths"][0],
        "legal/hold/**"
    );

    // A step's own deny is still authored onto the skill unit it enacts (RFC-0029) —
    // the playbook deny composes with it by union, it does not replace it.
    let purge = units
        .iter()
        .find(|u| u["id"] == "purge-records")
        .expect("the purge-records skill unit");
    assert_eq!(purge["action_scope"]["deny"]["tools"][0], "shell");
}

#[test]
fn author_playbook_omits_deny_when_spec_has_none() {
    // Absent deny is a no-op: the deny-free spec authors byte-for-byte what it did
    // before RFC-0030 — no `deny` key appears anywhere in the playbook unit.
    let dir = TempDir::new().unwrap();
    write(&dir, "promote.playbook.yaml", SPEC);
    forge()
        .arg("author-playbook")
        .arg("--apply")
        .arg(dir.path())
        .assert()
        .success();
    let m: serde_yaml::Value =
        serde_yaml::from_str(&fs::read_to_string(dir.path().join(SIBLING)).unwrap()).unwrap();
    let playbook = m["units"]
        .as_sequence()
        .unwrap()
        .iter()
        .find(|u| u["kind"] == "playbook")
        .unwrap();
    assert!(
        playbook["action_scope"]["deny"].is_null(),
        "no deny declared → no deny emitted"
    );
}

#[test]
fn author_playbook_warns_on_a_self_nullified_step() {
    // RFC-0030 validator rule: a step whose skill's whole allowlist for a dimension is
    // contained in the EFFECTIVE deny (playbook ∪ skill) is self-nullified — it reads
    // enactable but cannot act. Neither source alone contains the allow here; only the
    // union does, so this also pins the union semantics. SHOULD warn, never fail.
    let dir = TempDir::new().unwrap();
    write(
        &dir,
        "dead.playbook.yaml",
        "name: dead-step\ndescription: the union of denies refuses every tool the step allows\ndeny:\n  tools: [git]\nsteps:\n  - id: s1\n    uses: worker\n    tools: [git, shell]\n    deny:\n      tools: [shell]\n",
    );
    forge()
        .arg("author-playbook")
        .arg("--apply")
        .arg(dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("self-nullified-step"));
    assert!(
        dir.path().join("dead-step.playbook.kcp.yaml").exists(),
        "a self-nullified step warns; it does not refuse authoring"
    );
}

#[test]
fn author_playbook_notes_an_empty_playbook_deny() {
    // §4.3a: an empty deny object prohibits nothing. The lint names it, and the no-op
    // is never written into the artifact.
    let dir = TempDir::new().unwrap();
    write(
        &dir,
        "noop.playbook.yaml",
        "name: noop-deny\ndescription: a deny that lists nothing\ndeny:\n  tools: []\nsteps:\n  - id: s1\n    uses: a\n    tools: [git]\n",
    );
    forge()
        .arg("author-playbook")
        .arg("--apply")
        .arg(dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("empty-deny"));
    let m: serde_yaml::Value = serde_yaml::from_str(
        &fs::read_to_string(dir.path().join("noop-deny.playbook.kcp.yaml")).unwrap(),
    )
    .unwrap();
    let playbook = m["units"]
        .as_sequence()
        .unwrap()
        .iter()
        .find(|u| u["kind"] == "playbook")
        .unwrap();
    assert!(
        playbook["action_scope"]["deny"].is_null(),
        "an empty deny is a no-op and must not be emitted"
    );
}

#[test]
fn author_playbook_refuses_to_overwrite_a_hand_edited_artifact() {
    // Mirror convert's rule 4: destructive operations refuse when unsure.
    let dir = TempDir::new().unwrap();
    write(&dir, "promote.playbook.yaml", SPEC);
    forge()
        .arg("author-playbook")
        .arg("--apply")
        .arg(dir.path())
        .assert()
        .success();
    let sibling = dir.path().join(SIBLING);
    let mut edited = fs::read_to_string(&sibling).unwrap();
    edited.push_str("# hand edit that must not be lost\n");
    fs::write(&sibling, &edited).unwrap();
    write(&dir, "promote.playbook.yaml", &format!("{SPEC}# changed\n"));
    forge()
        .arg("author-playbook")
        .arg("--apply")
        .arg(dir.path())
        .assert()
        .code(1)
        .stdout(predicate::str::contains("hand-edited"));
    let after = fs::read_to_string(&sibling).unwrap();
    assert!(
        after.contains("hand edit that must not be lost"),
        "hand edit clobbered"
    );
}
