//! `kcp-forge validate` — every fixture here reproduces a failure measured on a real
//! ~680-skill corpus on 2026-07-30. If a fixture looks contrived, it happened.

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

const GOOD: &str =
    "name: good-skill\ndescription: a healthy skill\ninstructions: |\n  do the thing\n";

#[test]
fn clean_corpus_exits_zero() {
    let dir = TempDir::new().unwrap();
    write(&dir, "good-skill.yaml", GOOD);
    forge()
        .arg("validate")
        .arg(dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("checked 1 files"));
}

#[test]
fn invalid_yaml_is_a_problem_and_exits_one() {
    let dir = TempDir::new().unwrap();
    // The auto-reflect tail: appended at two-space indent after the last key, which
    // YAML reads as a continuation of the preceding list. 27 files rotted this way.
    write(
        &dir,
        "broken.yaml",
        "name: broken\ndescription: d\ntags:\n  - a\n\n  ## Reflected 2026-07-30\n  a substantial note that breaks the parse\n",
    );
    forge()
        .arg("validate")
        .arg(dir.path())
        .assert()
        .code(1)
        .stdout(predicate::str::contains("invalid-yaml"));
}

#[test]
fn name_mismatch_is_a_problem_and_fix_names_repairs_it() {
    let dir = TempDir::new().unwrap();
    write(
        &dir,
        "real-name.yaml",
        "name: declared-name\ndescription: d\ninstructions: |\n  body\n",
    );
    forge()
        .arg("validate")
        .arg(dir.path())
        .assert()
        .code(1)
        .stdout(predicate::str::contains("name-mismatch"));
    // --fix-names rewrites name: to the filename (the addressable identity), then clean.
    forge()
        .arg("validate")
        .arg("--fix-names")
        .arg(dir.path())
        .assert()
        .success();
    let fixed = fs::read_to_string(dir.path().join("real-name.yaml")).unwrap();
    assert!(
        fixed.contains("name: real-name"),
        "name not rewritten: {fixed}"
    );
}

#[test]
fn alternative_body_schemas_are_notes_not_problems() {
    // The corpus genuinely has four body conventions. A gate that assumes one flagged
    // 23-53 healthy files across five wrong iterations. These must all pass.
    let dir = TempDir::new().unwrap();
    write(
        &dir,
        "a.yaml",
        "name: a\ndescription: d\ninstructions: |\n  x\n",
    );
    write(
        &dir,
        "b.yaml",
        "name: b\ndescription: d\ninvoke:\n  prompt: |\n    x\n",
    );
    write(&dir, "c.yaml", "name: c\ndescription: d\ncontent: |\n  x\n");
    write(&dir, "d.yaml", "name: d\ndescription: d\nprompt: |\n  x\n");
    // Bespoke shape: body under keys the tool has never seen. A note, never a failure.
    write(
        &dir,
        "e.yaml",
        "name: e\ndescription: d\nrecipes:\n  - step one\nwhy_this_skill_exists: because\n",
    );
    forge()
        .arg("validate")
        .arg(dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("other-schema"));
}

#[test]
fn metadata_only_file_is_empty_problem() {
    let dir = TempDir::new().unwrap();
    write(
        &dir,
        "hollow.yaml",
        "name: hollow\ndescription: all hat no cattle\nversion: 1.0.0\ntags: [x]\n",
    );
    forge()
        .arg("validate")
        .arg(dir.path())
        .assert()
        .code(1)
        .stdout(predicate::str::contains("empty"));
}

#[test]
fn kcp_manifest_in_corpus_is_a_note_not_a_skill_failure() {
    let dir = TempDir::new().unwrap();
    write(&dir, "good-skill.yaml", GOOD);
    // Three knowledge.yaml manifests sat in the real skills directory. Not defects.
    write(
        &dir,
        "spike.knowledge.yaml",
        "project: spike\nkcp_version: \"0.30\"\nunits:\n  - id: u\n    path: good-skill.yaml\n    intent: i\n",
    );
    forge()
        .arg("validate")
        .arg(dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("not-a-skill"));
}

#[test]
fn dangling_see_also_is_a_problem_and_subdir_targets_resolve() {
    let dir = TempDir::new().unwrap();
    // The real corpus keeps skills in subdirectories too; a scanner that saw one level
    // reported healthy cross-references as dangling. `common/target` must resolve.
    write(
        &dir,
        "refs.yaml",
        "name: refs\ndescription: d\ninstructions: |\n  x\nsee_also:\n  - lives-in-subdir\n  - truly-missing\n",
    );
    write(
        &dir,
        "common/lives-in-subdir.yaml",
        "name: lives-in-subdir\ndescription: d\ninstructions: |\n  x\n",
    );
    forge()
        .arg("validate")
        .arg(dir.path())
        .assert()
        .code(1)
        .stdout(
            predicate::str::contains("truly-missing")
                .and(predicate::str::contains("lives-in-subdir (no such skill)").not()),
        );
}

#[test]
fn json_mode_is_machine_readable() {
    let dir = TempDir::new().unwrap();
    write(&dir, "broken.yaml", "name: [unclosed\n");
    let out = forge()
        .arg("validate")
        .arg("--json")
        .arg(dir.path())
        .assert()
        .code(1)
        .get_output()
        .stdout
        .clone();
    let v: serde_json::Value = serde_json::from_slice(&out).expect("stdout must be valid JSON");
    assert_eq!(v["checked"], 1);
    assert_eq!(v["findings"][0]["kind"], "invalid-yaml");
    assert_eq!(v["findings"][0]["severity"], "problem");
}

#[test]
fn register_and_archives_are_not_corpus_members() {
    let dir = TempDir::new().unwrap();
    write(&dir, "good-skill.yaml", GOOD);
    write(&dir, "skill-register.yaml", "not: [valid: yaml: at: all\n");
    write(&dir, "archive/old.yaml", "broken: [\n");
    write(&dir, "archive-2026-02-03/old.yaml", "broken: [\n");
    forge().arg("validate").arg(dir.path()).assert().success();
}

#[test]
fn corpus_inside_a_dotted_root_is_still_discovered() {
    // The real corpus lives at ~/.claude/skills. An exclusion meant for `.claude/`
    // subdirectories inside repos matched the root's own ancestry, silently excluded
    // all 681 files, and reported clean. Exclusions apply below the root, never to it.
    let dir = TempDir::new().unwrap();
    write(&dir, ".claude/skills/good-skill.yaml", GOOD);
    forge()
        .arg("validate")
        .arg(dir.path().join(".claude/skills"))
        .assert()
        .success()
        .stdout(predicate::str::contains("checked 1 files"));
}

#[test]
fn empty_corpus_is_a_problem_not_a_clean_pass() {
    // A gate pointed at nothing must not report clean — silence is never evidence.
    let dir = TempDir::new().unwrap();
    forge()
        .arg("validate")
        .arg(dir.path())
        .assert()
        .code(1)
        .stdout(predicate::str::contains("empty-corpus"));
}
