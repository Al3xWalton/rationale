//! User-facing command-line entry point for local Rationale evidence.

use std::{path::PathBuf, process::ExitCode};

use clap::{Parser, Subcommand};
use rationale_engine::{Engine, EngineError, WhyResult};
use rationale_model::{EvidenceNode, KernelResponse, Verdict};

const EXIT_OK: u8 = 0;
const EXIT_OPERATIONAL: u8 = 1;
const EXIT_MISSING_RATIONALE: u8 = 2;
const EXIT_CONFLICT: u8 = 3;
const EXIT_INVALID_INPUT: u8 = 4;
const EXIT_STALE: u8 = 5;
const EXIT_PROOF_UNAVAILABLE: u8 = 6;

#[derive(Debug, Parser)]
#[command(name = "rationale", version, about = "Prove why code exists")]
struct Cli {
    #[arg(long, global = true, value_name = "PATH")]
    database: Option<PathBuf>,
    #[arg(long, global = true, value_name = "PATH")]
    worker: Option<PathBuf>,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Synchronize explicit evidence sources.
    Sync {
        /// Use only local Git and repository documents.
        #[arg(long)]
        local: bool,
        /// Emit stable machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Explain the recorded rationale for a target.
    Why {
        /// `path:line`, `path:start-end@revision`, or `commit:revision`.
        target: String,
        /// Emit stable machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Show one normalized current evidence record.
    Show {
        /// Stable record identifier.
        record_id: String,
        /// Emit stable machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Inspect known rationale gaps.
    Gaps {
        /// Report changed working-copy paths without inferred attribution.
        #[arg(long, required = true)]
        changed: bool,
        /// Emit stable machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Serve the read-only local tools over MCP stdio.
    Serve,
}

impl Commands {
    const fn json(&self) -> bool {
        match self {
            Self::Sync { json, .. }
            | Self::Why { json, .. }
            | Self::Show { json, .. }
            | Self::Gaps { json, .. } => *json,
            Self::Serve => false,
        }
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    let json = cli.command.json();
    match run(cli).await {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            render_error(&error, json);
            ExitCode::from(error_exit(&error))
        }
    }
}

async fn run(cli: Cli) -> Result<u8, EngineError> {
    let engine = Engine::discover(".", cli.database)?.with_worker_path(cli.worker);
    match cli.command {
        Commands::Sync { local, json } => run_sync(&engine, local, json).await,
        Commands::Why { target, json } => {
            let result = engine.why(&target).await?;
            if json {
                print_json(&result);
            } else {
                print_why(&result);
            }
            Ok(why_exit(&result))
        }
        Commands::Show { record_id, json } => {
            if let Some(record) = engine.show(&record_id)? {
                if json {
                    print_json(&record);
                } else {
                    print_record(&record);
                }
                Ok(EXIT_OK)
            } else {
                if json {
                    print_json(&serde_json::json!({
                        "status": "missing",
                        "record_id": record_id
                    }));
                } else {
                    println!("Record not found: {record_id}");
                }
                Ok(EXIT_MISSING_RATIONALE)
            }
        }
        Commands::Gaps { changed, json } => {
            debug_assert!(changed, "clap requires the changed-path mode");
            let gaps = engine.gaps_changed()?;
            if json {
                print_json(&serde_json::json!({ "gaps": gaps }));
            } else if gaps.is_empty() {
                println!("Changed gaps: none");
            } else {
                println!("Changed gaps:");
                for gap in &gaps {
                    println!("  {} — {}", gap.path, gap.code);
                }
            }
            Ok(if gaps.is_empty() {
                EXIT_OK
            } else {
                EXIT_MISSING_RATIONALE
            })
        }
        Commands::Serve => {
            rationale_mcp::serve(engine)
                .await
                .map_err(|error| std::io::Error::other(error.to_string()))?;
            Ok(EXIT_OK)
        }
    }
}

async fn run_sync(engine: &Engine, local: bool, json: bool) -> Result<u8, EngineError> {
    let report = if local {
        engine.sync_local()?
    } else {
        engine.sync().await?
    };
    if json {
        print_json(&report);
    } else {
        println!("Snapshot: {}", report.snapshot_id);
        println!(
            "Evidence: {} records, {} edges, {} conflicts",
            report.records, report.edges, report.conflicts
        );
        println!("Quarantined: {}", report.quarantined);
        if !report.stale_sources.is_empty() {
            println!("Stale sources: {}", report.stale_sources.join(", "));
        }
        if let Some(rate) = &report.github_rate_limit {
            println!(
                "GitHub requests: {} remaining of {}",
                rate.remaining
                    .map_or_else(|| "unknown".to_owned(), |value| value.to_string()),
                rate.limit
                    .map_or_else(|| "unknown".to_owned(), |value| value.to_string())
            );
        }
        println!(
            "Status: {}",
            if report.unchanged {
                "unchanged"
            } else {
                "published"
            }
        );
    }
    Ok(if report.stale_sources.is_empty() {
        EXIT_OK
    } else {
        EXIT_STALE
    })
}

fn print_json(value: &impl serde::Serialize) {
    println!(
        "{}",
        serde_json::to_string_pretty(value).expect("public output models must serialize")
    );
}

fn print_why(result: &WhyResult) {
    match &result.response {
        KernelResponse::Proof { proof, .. } => {
            println!("Verdict: {}", json_name(&proof.verdict));
            println!("Proof paths:");
            if proof.proof_chains.is_empty() {
                println!("  none");
            } else {
                for chain in &proof.proof_chains {
                    println!("  {}", chain.node_ids.join(" -> "));
                }
            }
            println!("Gaps:");
            if proof.gaps.is_empty() {
                println!("  none");
            } else {
                for gap in &proof.gaps {
                    println!(
                        "  {} from {}",
                        json_name(&gap.code),
                        gap.from_node_id.as_deref().unwrap_or("unknown")
                    );
                }
            }
            if !proof.conflicts.is_empty() {
                println!("  conflicts:");
                for conflict in &proof.conflicts {
                    println!(
                        "    {} <> {} ({})",
                        conflict.left_node_id, conflict.right_node_id, conflict.rule_id
                    );
                }
            }
        }
        KernelResponse::Error { error, .. } => {
            println!("Verdict: unavailable");
            println!("Proof paths:\n  none");
            println!("Gaps:\n  {} — {}", json_name(&error.code), error.message);
        }
    }
    println!("Candidates:");
    if result.candidates.is_empty() {
        println!("  none");
    } else {
        for candidate in &result.candidates {
            println!(
                "  {} [v{} score {}] — {}",
                candidate.record_id, candidate.score_version, candidate.score, candidate.reason
            );
        }
    }
    println!("Freshness:");
    for source in &result.freshness {
        println!(
            "  {}: {} at {}",
            source.source_id, source.freshness, source.observed_at
        );
    }
}

fn print_record(record: &EvidenceNode) {
    println!("Record: {}", record.id);
    println!("Kind: {}", json_name(&record.kind));
    println!("Status: {}", json_name(&record.status));
    println!("Origin: {}", record.origin.locator);
    if let Some(revision) = &record.origin.revision {
        println!("Revision: {revision}");
    }
}

fn json_name(value: &impl serde::Serialize) -> String {
    serde_json::to_string(value)
        .expect("protocol enum must serialize")
        .trim_matches('"')
        .to_owned()
}

fn why_exit(result: &WhyResult) -> u8 {
    if result.has_stale_source() {
        return EXIT_STALE;
    }
    match &result.response {
        KernelResponse::Proof { proof, .. } => match proof.verdict {
            Verdict::Established => EXIT_OK,
            Verdict::Partial | Verdict::NotEstablished => EXIT_MISSING_RATIONALE,
            Verdict::Conflicted => EXIT_CONFLICT,
        },
        KernelResponse::Error { .. } => EXIT_INVALID_INPUT,
    }
}

fn error_exit(error: &EngineError) -> u8 {
    match error {
        EngineError::ProofUnavailable(_) => EXIT_PROOF_UNAVAILABLE,
        EngineError::Git(_) | EngineError::GitHub(_) | EngineError::InvalidQuery { .. } => {
            EXIT_INVALID_INPUT
        }
        EngineError::NoSnapshot => EXIT_MISSING_RATIONALE,
        EngineError::Store(_) | EngineError::Io(_) | EngineError::ScanLimit { .. } => {
            EXIT_OPERATIONAL
        }
    }
}

fn render_error(error: &EngineError, json: bool) {
    if json {
        print_json(&serde_json::json!({
            "status": "error",
            "error": {
                "category": match error_exit(error) {
                    EXIT_PROOF_UNAVAILABLE => "proof_unavailable",
                    EXIT_INVALID_INPUT => "invalid_input",
                    EXIT_MISSING_RATIONALE => "missing_rationale",
                    _ => "operational",
                },
                "message": error.to_string()
            }
        }));
    } else {
        eprintln!("rationale: {error}");
    }
}
