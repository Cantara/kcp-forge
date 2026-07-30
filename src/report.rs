//! Findings model shared by every subcommand.
//!
//! Two severities only. `Problem` fails the run (exit 1); `Note` is informational and
//! never fails. The distinction is the design's rule 3: fail on universal truths,
//! report shape diversity — a gate that cries wolf on healthy files teaches people to
//! ignore it.

use serde::Serialize;

#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Problem,
    Note,
}

#[derive(Serialize, Debug, Clone)]
pub struct Finding {
    pub severity: Severity,
    /// Stable machine key, e.g. "invalid-yaml", "name-mismatch", "other-schema"
    pub kind: String,
    /// Repo-relative path of the file concerned
    pub file: String,
    pub message: String,
}

#[derive(Serialize, Debug, Default)]
pub struct Report {
    pub checked: usize,
    pub findings: Vec<Finding>,
}

impl Report {
    pub fn problem(&mut self, kind: &str, file: &str, message: impl Into<String>) {
        self.findings.push(Finding {
            severity: Severity::Problem,
            kind: kind.to_string(),
            file: file.to_string(),
            message: message.into(),
        });
    }

    pub fn note(&mut self, kind: &str, file: &str, message: impl Into<String>) {
        self.findings.push(Finding {
            severity: Severity::Note,
            kind: kind.to_string(),
            file: file.to_string(),
            message: message.into(),
        });
    }

    pub fn problems(&self) -> impl Iterator<Item = &Finding> {
        self.findings
            .iter()
            .filter(|f| f.severity == Severity::Problem)
    }

    pub fn clean(&self) -> bool {
        self.problems().count() == 0
    }

    /// Render to stdout. Text mode groups problems before notes so the failing class is
    /// never buried; JSON mode is the full structure, verbatim.
    pub fn emit(&self, json: bool) {
        if json {
            println!(
                "{}",
                serde_json::to_string_pretty(self).expect("report serializes")
            );
            return;
        }
        println!("checked {} files", self.checked);
        let problems: Vec<_> = self.problems().collect();
        if !problems.is_empty() {
            println!();
            for f in &problems {
                println!("PROBLEM {:<16} {}: {}", f.kind, f.file, f.message);
            }
        }
        let notes: Vec<_> = self
            .findings
            .iter()
            .filter(|f| f.severity == Severity::Note)
            .collect();
        if !notes.is_empty() {
            println!();
            for f in &notes {
                println!("note    {:<16} {}: {}", f.kind, f.file, f.message);
            }
        }
        println!();
        if problems.is_empty() {
            println!("clean");
        } else {
            println!("{} problem(s)", problems.len());
        }
    }
}
