//! Corpus discovery. Skills live in more than one place — a top-level directory and
//! subdirectories like `common/` — and a scanner that sees only one level reported
//! "0 invalid" while 15 of 19 files in a subdirectory were unparseable. Recurse, and
//! exclude only what is deliberately not live.

use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// Files that are infrastructure or archive, never corpus members.
fn excluded(path: &Path) -> bool {
    path.components().any(|c| {
        let s = c.as_os_str().to_string_lossy();
        s.starts_with("archive")
            || s == "__pycache__"
            || s == "node_modules"
            || s == ".git"
            || s == "target"
            || s == "examples"
            || s.starts_with(".claude")
    }) || path
        .file_name()
        .map(|n| {
            let n = n.to_string_lossy();
            n.starts_with("reflect-") || n == "skill-register.yaml"
        })
        .unwrap_or(false)
}

/// True for a derived sibling written by `convert` — never validated as a source skill.
pub fn is_sibling(path: &Path) -> bool {
    path.file_name()
        .map(|n| n.to_string_lossy().ends_with(".kcp.yaml"))
        .unwrap_or(false)
}

/// Every live YAML file under the given roots (default `.`), sorted, siblings included.
pub fn discover(paths: &[PathBuf]) -> Vec<PathBuf> {
    let roots: Vec<PathBuf> = if paths.is_empty() {
        vec![PathBuf::from(".")]
    } else {
        paths.to_vec()
    };
    let mut out = Vec::new();
    for root in roots {
        if root.is_file() {
            // An explicitly named file is the operator's assertion; only its own name
            // can exclude it, never its ancestry.
            let name_only = root
                .file_name()
                .map(std::path::PathBuf::from)
                .unwrap_or_default();
            if !excluded(&name_only) {
                out.push(root);
            }
            continue;
        }
        for entry in WalkDir::new(&root)
            .follow_links(false)
            .into_iter()
            .flatten()
        {
            let p = entry.path();
            // Exclusions apply to the path BELOW the root, never to the root's own
            // ancestry. The real corpus lives at ~/.claude/skills — judging the full
            // path excluded all 681 files and reported a clean empty scan.
            let rel = p.strip_prefix(&root).unwrap_or(p);
            if p.is_file()
                && p.extension()
                    .map(|e| e == "yaml" || e == "yml")
                    .unwrap_or(false)
                && !excluded(rel)
            {
                out.push(p.to_path_buf());
            }
        }
    }
    out.sort();
    out
}
