//! `kcp-forge convert` and `kcp-forge drift` — the sibling model.
//!
//! The precedent: a 644-unit skill→KCP conversion that rotted three ways, silently.
//! Frozen at kcp_version 0.7 against a 0.30 spec; no eligibility grants, so every
//! governed unit failed closed; 34 skills added later never entered it. Each drift
//! signal below exists because one of those happened.

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

const SKILL: &str = "name: deploy-runbook\ndescription: how to deploy the thing safely\ntrigger_phrases:\n  - \"deploy\"\n  - \"release the thing\"\ninstructions: |\n  step one, step two\n";

#[test]
fn convert_is_dry_run_by_default() {
    let dir = TempDir::new().unwrap();
    write(&dir, "deploy-runbook.yaml", SKILL);
    forge()
        .arg("convert")
        .arg(dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("would write"));
    assert!(
        !dir.path().join("deploy-runbook.kcp.yaml").exists(),
        "dry-run must not write"
    );
}

#[test]
fn convert_apply_writes_a_governed_sibling() {
    let dir = TempDir::new().unwrap();
    write(&dir, "deploy-runbook.yaml", SKILL);
    forge()
        .arg("convert")
        .arg("--apply")
        .arg(dir.path())
        .assert()
        .success();

    let sibling = dir.path().join("deploy-runbook.kcp.yaml");
    assert!(sibling.exists(), "sibling not written");
    let unit: serde_yaml::Value =
        serde_yaml::from_str(&fs::read_to_string(&sibling).unwrap()).unwrap();

    // The governed triple: without any one of these, the planner either does not see a
    // procedure at all, or fails it closed as not invoke-eligible (RFC-0028).
    assert_eq!(unit["kind"], "skill");
    assert_eq!(unit["load_eligible"], true);
    assert_eq!(unit["id"], "deploy-runbook");
    // Derived, not invented: intent from description, triggers from trigger_phrases.
    assert!(
        unit["intent"]
            .as_str()
            .unwrap()
            .contains("deploy the thing")
    );
    assert_eq!(unit["triggers"][0], "deploy");
    // The drift tether: a sha256 over the SOURCE file.
    assert_eq!(unit["content_hash"]["algorithm"], "sha256");
    assert_eq!(unit["content_hash"]["value"].as_str().unwrap().len(), 64);
    // Provenance: which converter, from what, against which spec.
    assert!(unit["x-forge"]["kcp_version"].as_str().is_some());
    assert_eq!(unit["x-forge"]["source"], "deploy-runbook.yaml");
}

#[test]
fn convert_never_converts_siblings_or_manifests() {
    let dir = TempDir::new().unwrap();
    write(&dir, "deploy-runbook.yaml", SKILL);
    forge()
        .arg("convert")
        .arg("--apply")
        .arg(dir.path())
        .assert()
        .success();
    // Second run over the same tree: the sibling itself must not be treated as a source
    // (no deploy-runbook.kcp.kcp.yaml), and a manifest is not a skill.
    write(
        &dir,
        "spike.knowledge.yaml",
        "project: spike\nkcp_version: \"0.30\"\nunits: []\n",
    );
    forge()
        .arg("convert")
        .arg("--apply")
        .arg(dir.path())
        .assert()
        .success();
    assert!(!dir.path().join("deploy-runbook.kcp.kcp.yaml").exists());
    assert!(!dir.path().join("spike.knowledge.kcp.yaml").exists());
}

#[test]
fn drift_clean_when_sibling_is_current() {
    let dir = TempDir::new().unwrap();
    write(&dir, "deploy-runbook.yaml", SKILL);
    forge()
        .arg("convert")
        .arg("--apply")
        .arg(dir.path())
        .assert()
        .success();
    forge()
        .arg("drift")
        .arg(dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("clean"));
}

#[test]
fn drift_detects_edited_source() {
    let dir = TempDir::new().unwrap();
    write(&dir, "deploy-runbook.yaml", SKILL);
    forge()
        .arg("convert")
        .arg("--apply")
        .arg(dir.path())
        .assert()
        .success();
    // Source edited after conversion → content_hash no longer matches.
    write(&dir, "deploy-runbook.yaml", &format!("{SKILL}# edited\n"));
    forge()
        .arg("drift")
        .arg(dir.path())
        .assert()
        .code(1)
        .stdout(predicate::str::contains("source-edited"));
}

#[test]
fn drift_detects_unconverted_source() {
    // The additive-drift mode: 34 skills were added after the real conversion ran, and
    // nothing noticed. A source with no sibling is a finding, not an absence.
    let dir = TempDir::new().unwrap();
    write(&dir, "deploy-runbook.yaml", SKILL);
    forge()
        .arg("convert")
        .arg("--apply")
        .arg(dir.path())
        .assert()
        .success();
    write(
        &dir,
        "newcomer.yaml",
        "name: newcomer\ndescription: d\ninstructions: |\n  x\n",
    );
    forge()
        .arg("drift")
        .arg(dir.path())
        .assert()
        .code(1)
        .stdout(predicate::str::contains("unconverted"));
}

#[test]
fn drift_detects_spec_drift_in_sibling() {
    // The 0.7-freeze mode: a sibling declaring an older kcp_version than the converter
    // targets is stale even if its hash still matches.
    let dir = TempDir::new().unwrap();
    write(&dir, "deploy-runbook.yaml", SKILL);
    forge()
        .arg("convert")
        .arg("--apply")
        .arg(dir.path())
        .assert()
        .success();
    let sibling = dir.path().join("deploy-runbook.kcp.yaml");
    let stale = fs::read_to_string(&sibling)
        .unwrap()
        .lines()
        .map(|l| {
            if l.trim_start().starts_with("kcp_version:") {
                "  kcp_version: \"0.7\"".to_string()
            } else {
                l.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(&sibling, stale).unwrap();
    forge()
        .arg("drift")
        .arg(dir.path())
        .assert()
        .code(1)
        .stdout(predicate::str::contains("spec-drift"));
}

#[test]
fn convert_refuses_to_overwrite_a_hand_edited_sibling() {
    // Rule 4: destructive operations refuse when unsure. If someone edited the sibling
    // by hand (its recorded self-integrity no longer holds), convert --apply must not
    // clobber their work silently.
    let dir = TempDir::new().unwrap();
    write(&dir, "deploy-runbook.yaml", SKILL);
    forge()
        .arg("convert")
        .arg("--apply")
        .arg(dir.path())
        .assert()
        .success();
    let sibling = dir.path().join("deploy-runbook.kcp.yaml");
    let mut edited = fs::read_to_string(&sibling).unwrap();
    edited.push_str("# hand edit that must not be lost\n");
    fs::write(&sibling, edited).unwrap();
    // Also edit the source so convert would want to regenerate.
    write(&dir, "deploy-runbook.yaml", &format!("{SKILL}# changed\n"));
    forge()
        .arg("convert")
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

#[test]
fn drift_json_is_machine_readable() {
    let dir = TempDir::new().unwrap();
    write(
        &dir,
        "a.yaml",
        "name: a\ndescription: d\ninstructions: |\n  x\n",
    );
    let out = forge()
        .arg("drift")
        .arg("--json")
        .arg(dir.path())
        .assert()
        .code(1)
        .get_output()
        .stdout
        .clone();
    let v: serde_json::Value = serde_json::from_slice(&out).expect("stdout must be valid JSON");
    assert_eq!(v["findings"][0]["kind"], "unconverted");
}
