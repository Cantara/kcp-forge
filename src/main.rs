//! kcp-forge — corpus-level QA for KCP knowledge and skills.
//!
//! Exit codes are the contract: 0 = clean, 1 = findings, 2 = tool error.
//! Every subcommand supports `--json`; destructive subcommands are dry-run by default.
//! See DESIGN.md for the rules and the measured failures that bought them.

use clap::{Parser, Subcommand};
use std::process::ExitCode;

mod convert;
mod corpus;
mod playbook;
mod report;
mod schema;
mod validate;

#[derive(Parser)]
#[command(name = "kcp-forge", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Check corpus structural integrity (read-only)
    Validate {
        /// Corpus directories or files (default: current directory)
        paths: Vec<std::path::PathBuf>,
        /// Machine-readable output
        #[arg(long)]
        json: bool,
        /// Rewrite `name:` to match the filename where they disagree
        #[arg(long)]
        fix_names: bool,
    },
    /// Derive a governed KCP unit sibling (<name>.kcp.yaml) per skill file
    Convert {
        paths: Vec<std::path::PathBuf>,
        #[arg(long)]
        json: bool,
        /// Write the siblings (default: dry-run, show what would be written)
        #[arg(long)]
        apply: bool,
    },
    /// Report how converted siblings have drifted from their sources (read-only)
    Drift {
        paths: Vec<std::path::PathBuf>,
        #[arg(long)]
        json: bool,
    },
    /// Assemble a governed `kind: playbook` manifest from a spec (a YAML file declaring
    /// `steps:`); validated against the real KCP schema before writing
    AuthorPlaybook {
        paths: Vec<std::path::PathBuf>,
        #[arg(long)]
        json: bool,
        /// Write the manifest (default: dry-run, show what would be written)
        #[arg(long)]
        apply: bool,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Validate {
            paths,
            json,
            fix_names,
        } => validate::run(&paths, json, fix_names),
        Command::Convert { paths, json, apply } => convert::run_convert(&paths, json, apply),
        Command::Drift { paths, json } => convert::run_drift(&paths, json),
        Command::AuthorPlaybook { paths, json, apply } => playbook::run(&paths, json, apply),
    };
    match result {
        Ok(clean) => {
            if clean {
                ExitCode::from(0)
            } else {
                ExitCode::from(1)
            }
        }
        Err(err) => {
            eprintln!("kcp-forge: {err:#}");
            ExitCode::from(2)
        }
    }
}
