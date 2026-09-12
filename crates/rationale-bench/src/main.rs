//! Reproducible end-to-end performance budgets for Rationale.

use std::{
    collections::BTreeSet,
    error::Error,
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::Instant,
};

use clap::Parser;
use fixture::{GeneratedRepository, Scale};
use rationale_engine::Engine;
use rationale_model::KernelRequest;
use rationale_protocol::{WorkerConfig, WorkerSupervisor, decode_frame, to_canonical_json};
use rationale_store::{CandidateMetadata, EvidenceStore, GraphSlice, SliceLimits};
use report::{BenchmarkReport, Budgets, CheckStatus, Metrics, ProofScope, Stability};
use rmcp::{
    ServiceExt,
    model::CallToolRequestParams,
    transport::{ConfigureCommandExt, TokioChildProcess},
};
use serde_json::Value;
use stats::{Distribution, host_metadata, resident_tree_kib, rounded};

mod fixture;
mod ocaml;
mod report;
mod stats;

type BenchError = Box<dyn Error + Send + Sync>;
type BenchResult<T> = Result<T, BenchError>;

const SLICE_LIMITS: SliceLimits = SliceLimits {
    max_nodes: 4_000,
    max_edges: 16_384,
    max_conflicts: 4_096,
};

struct QueryContext<'a> {
    cli: &'a Path,
    worker: &'a Path,
    database: &'a Path,
    repository: &'a Path,
    target: &'a str,
    engine: &'a Engine,
    request: &'a KernelRequest,
}

struct QueryMeasurements {
    worker_round_trip: Distribution,
    warm_engine_query: Distribution,
    warm_cli_query: Distribution,
    warm_mcp_query: Distribution,
    rss_before_kib: u64,
    rss_after_kib: u64,
    engine_stable: CheckStatus,
    cli_stable: CheckStatus,
    mcp_stable: CheckStatus,
    worker_stable: CheckStatus,
}

#[derive(Debug, Parser)]
#[command(about = "Measure Rationale's generated-scale performance budgets")]
struct Arguments {
    #[arg(long, value_enum, default_value = "ava-like")]
    scale: Scale,
    #[arg(long, default_value_t = 30)]
    samples: usize,
    #[arg(long, default_value_t = 200)]
    repeated_queries: usize,
    #[arg(long, default_value = "target/release/rationale")]
    cli: PathBuf,
    #[arg(long, default_value = "ocaml/_build/default/worker/main.exe")]
    worker: PathBuf,
    #[arg(long, default_value = "ocaml/_build/default/bench/main.exe")]
    ocaml_bench: PathBuf,
    #[arg(long, default_value = "benchmarks/results/latest.json")]
    output: PathBuf,
    #[arg(long)]
    keep_repository: bool,
    #[arg(long)]
    no_enforce: bool,
}

#[tokio::main]
async fn main() -> BenchResult<()> {
    let arguments = Arguments::parse();
    let root = repository_root()?;
    let report = run(&root, &arguments).await?;
    let output = resolve(&root, &arguments.output);
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(
        &output,
        format!("{}\n", serde_json::to_string_pretty(&report)?),
    )?;
    println!("{}", output.display());
    if !arguments.no_enforce && !report.budgets.all_passed.passed() {
        return Err(failure("one or more performance budgets failed"));
    }
    Ok(())
}

async fn run(root: &Path, arguments: &Arguments) -> BenchResult<BenchmarkReport> {
    let samples = arguments.samples.max(1);
    let repeated_queries = arguments.repeated_queries.max(1);
    let (cli, worker, ocaml_bench) = benchmark_executables(root, arguments)?;

    let mut repository = GeneratedRepository::create(arguments.scale, arguments.keep_repository)?;
    let database = repository.path().join(".rationale/benchmark.db");
    let engine = Engine::discover(repository.path(), Some(database.clone()))?
        .with_worker_path(Some(worker.clone()));
    let (cold_local_ingestion, incremental_local_sync) =
        benchmark_ingestion(&engine, &mut repository, samples)?;

    let head = repository.head()?;
    let (graph_slice_extraction, sqlite_publication, full, proof) =
        benchmark_storage(&database, &head, samples)?;
    let request = fixture_request(root)?;
    let rust_protocol_codec = benchmark_protocol_codec(&request, samples)?;
    let ocaml_result = ocaml::evaluate(
        &ocaml_bench,
        &root.join("fixtures/protocol/request-established.json"),
        samples.saturating_mul(1_000).max(10_000),
    )?;
    let queries = benchmark_queries(
        QueryContext {
            cli: &cli,
            worker: &worker,
            database: &database,
            repository: repository.path(),
            target: repository.target(),
            engine: &engine,
            request: &request,
        },
        samples,
        repeated_queries,
    )
    .await?;

    let estimated_process_protocol_overhead_ms =
        rounded((queries.worker_round_trip.median_ms - ocaml_result.per_evaluation_ms).max(0.0));
    let proof_scope = ProofScope {
        snapshot_nodes: full.nodes.len(),
        snapshot_edges: full.edges.len(),
        request_nodes: proof.nodes.len() + 1,
        request_edges: proof.edges.len() + 1,
    };
    let rss_growth_kib =
        i64::try_from(queries.rss_after_kib)? - i64::try_from(queries.rss_before_kib)?;
    let payloads_stable = queries.engine_stable.passed()
        && queries.cli_stable.passed()
        && queries.mcp_stable.passed()
        && queries.worker_stable.passed();
    let warm_budget = queries.warm_engine_query.p95_ms < 100.0
        && queries.warm_cli_query.p95_ms < 100.0
        && queries.warm_mcp_query.p95_ms < 100.0;
    let incremental_budget = incremental_local_sync.p95_ms < 1_000.0;
    let sliced = proof_scope.request_nodes < proof_scope.snapshot_nodes
        && proof_scope.request_edges < proof_scope.snapshot_edges;
    let memory_budget = rss_growth_kib <= 8_192;
    let all_passed =
        warm_budget && incremental_budget && sliced && memory_budget && payloads_stable;

    Ok(BenchmarkReport {
        schema_version: 1,
        scale: arguments.scale,
        fixture: arguments.scale.config(),
        measurement_samples: samples,
        host: host_metadata(root),
        metrics: Metrics {
            cold_local_ingestion,
            incremental_local_sync,
            graph_slice_extraction,
            rust_protocol_codec,
            ocaml_kernel_evaluation_ms: rounded(ocaml_result.per_evaluation_ms),
            worker_round_trip: queries.worker_round_trip,
            estimated_process_protocol_overhead_ms,
            warm_engine_query: queries.warm_engine_query,
            warm_cli_query: queries.warm_cli_query,
            warm_mcp_query: queries.warm_mcp_query,
            sqlite_publication,
        },
        proof_scope,
        stability: Stability {
            repeated_queries,
            rss_before_kib: queries.rss_before_kib,
            rss_after_kib: queries.rss_after_kib,
            rss_growth_kib,
            engine_payloads_byte_identical: queries.engine_stable,
            cli_payloads_byte_identical: queries.cli_stable,
            mcp_payloads_byte_identical: queries.mcp_stable,
            worker_payloads_byte_identical: queries.worker_stable,
        },
        budgets: Budgets {
            warm_query_p95_under_100_ms: warm_budget.into(),
            incremental_sync_p95_under_1000_ms: incremental_budget.into(),
            proof_request_is_sliced: sliced.into(),
            repeated_rss_growth_under_8192_kib: memory_budget.into(),
            canonical_payloads_stable: payloads_stable.into(),
            all_passed: all_passed.into(),
        },
    })
}

fn benchmark_ingestion(
    engine: &Engine,
    repository: &mut GeneratedRepository,
    samples: usize,
) -> BenchResult<(Distribution, Distribution)> {
    let started = Instant::now();
    engine.sync_local()?;
    let cold = Distribution::from_durations(&[started.elapsed()]);
    let mut incremental = Vec::with_capacity(samples);
    for _ in 0..samples {
        repository.commit_incremental()?;
        let started = Instant::now();
        engine.sync_local()?;
        incremental.push(started.elapsed());
    }
    Ok((cold, Distribution::from_durations(&incremental)))
}

fn benchmark_storage(
    database: &Path,
    head: &str,
    samples: usize,
) -> BenchResult<(Distribution, Distribution, GraphSlice, GraphSlice)> {
    let mut store = EvidenceStore::open(database)?;
    let full = store
        .load_current_slice(SLICE_LIMITS)?
        .ok_or_else(|| failure("benchmark snapshot is missing"))?;
    let seeds = [head.to_owned()];
    let mut durations = Vec::with_capacity(samples);
    let mut proof = None;
    for _ in 0..samples {
        let started = Instant::now();
        proof = store.load_current_proof_slice(&seeds, SLICE_LIMITS)?;
        durations.push(started.elapsed());
    }
    let proof = proof.ok_or_else(|| failure("benchmark proof slice is missing"))?;
    let publication = benchmark_publication(database, &full, samples)?;
    Ok((
        Distribution::from_durations(&durations),
        publication,
        full,
        proof,
    ))
}

fn benchmark_publication(
    source_database: &Path,
    graph: &GraphSlice,
    samples: usize,
) -> BenchResult<Distribution> {
    let database = source_database.with_file_name("publication-benchmark.db");
    let mut store = EvidenceStore::open(database)?;
    let mut durations = Vec::with_capacity(samples);
    for sample in 0..samples {
        let started = Instant::now();
        let candidate = store.begin_candidate(CandidateMetadata {
            id: format!("publication-{sample}"),
            created_at: "2026-09-12T00:00:00Z".to_owned(),
        })?;
        for node in &graph.nodes {
            candidate.insert_record(node)?;
        }
        for edge in &graph.edges {
            candidate.insert_edge(edge)?;
        }
        for conflict in &graph.conflicts {
            candidate.insert_conflict(conflict)?;
        }
        for source in &graph.sources {
            candidate.set_source_state(source)?;
        }
        candidate.publish("2026-09-12T00:00:01Z")?;
        durations.push(started.elapsed());
    }
    Ok(Distribution::from_durations(&durations))
}

fn benchmark_protocol_codec(request: &KernelRequest, samples: usize) -> BenchResult<Distribution> {
    const BATCH: u32 = 1_000;
    let mut durations = Vec::with_capacity(samples);
    for _ in 0..samples {
        let started = Instant::now();
        for _ in 0..BATCH {
            let payload = to_canonical_json(request)?;
            let length = u32::try_from(payload.len())?.to_be_bytes();
            let mut frame = Vec::with_capacity(payload.len() + 4);
            frame.extend_from_slice(&length);
            frame.extend_from_slice(payload.as_bytes());
            if decode_frame(&frame, 8 * 1024 * 1024)? != payload.as_bytes() {
                return Err(failure("protocol codec changed payload bytes"));
            }
        }
        durations.push(started.elapsed() / BATCH);
    }
    Ok(Distribution::from_durations(&durations))
}

async fn benchmark_queries(
    context: QueryContext<'_>,
    samples: usize,
    repeated_queries: usize,
) -> BenchResult<QueryMeasurements> {
    let (supervisor, worker_round_trip, worker_stable) =
        benchmark_worker(context.worker, context.request, samples).await?;
    let (warm_engine_query, engine_stable) =
        benchmark_engine(context.engine, context.target, samples).await?;
    let (warm_cli_query, cli_stable) = benchmark_cli(
        context.cli,
        context.worker,
        context.database,
        context.repository,
        context.target,
        samples,
    )?;
    let (warm_mcp_query, mcp_stable) = benchmark_mcp(
        context.cli,
        context.worker,
        context.database,
        context.repository,
        context.target,
        samples,
    )
    .await?;
    let (rss_before_kib, rss_after_kib) = memory_stability(
        context.engine,
        context.target,
        &supervisor,
        context.request,
        repeated_queries,
    )
    .await?;
    Ok(QueryMeasurements {
        worker_round_trip,
        warm_engine_query,
        warm_cli_query,
        warm_mcp_query,
        rss_before_kib,
        rss_after_kib,
        engine_stable: engine_stable.into(),
        cli_stable: cli_stable.into(),
        mcp_stable: mcp_stable.into(),
        worker_stable: worker_stable.into(),
    })
}

async fn benchmark_worker(
    worker: &Path,
    request: &KernelRequest,
    samples: usize,
) -> BenchResult<(WorkerSupervisor, Distribution, bool)> {
    let supervisor = WorkerSupervisor::start(WorkerConfig::new(worker)).await?;
    supervisor.evaluate(request.clone()).await?;
    let mut durations = Vec::with_capacity(samples);
    let mut payloads = BTreeSet::new();
    for _ in 0..samples {
        let started = Instant::now();
        let response = supervisor.evaluate(request.clone()).await?;
        durations.push(started.elapsed());
        payloads.insert(to_canonical_json(&response)?);
    }
    Ok((
        supervisor,
        Distribution::from_durations(&durations),
        payloads.len() == 1,
    ))
}

async fn benchmark_engine(
    engine: &Engine,
    target: &str,
    samples: usize,
) -> BenchResult<(Distribution, bool)> {
    engine.why(target).await?;
    let mut durations = Vec::with_capacity(samples);
    let mut payloads = BTreeSet::new();
    for _ in 0..samples {
        let started = Instant::now();
        let result = engine.why(target).await?;
        durations.push(started.elapsed());
        payloads.insert(to_canonical_json(&result)?);
    }
    Ok((
        Distribution::from_durations(&durations),
        payloads.len() == 1,
    ))
}

fn benchmark_cli(
    cli: &Path,
    worker: &Path,
    database: &Path,
    repository: &Path,
    target: &str,
    samples: usize,
) -> BenchResult<(Distribution, bool)> {
    run_cli(cli, worker, database, repository, target)?;
    let mut durations = Vec::with_capacity(samples);
    let mut payloads = BTreeSet::new();
    for _ in 0..samples {
        let started = Instant::now();
        let payload = run_cli(cli, worker, database, repository, target)?;
        durations.push(started.elapsed());
        payloads.insert(payload);
    }
    Ok((
        Distribution::from_durations(&durations),
        payloads.len() == 1,
    ))
}

fn run_cli(
    cli: &Path,
    worker: &Path,
    database: &Path,
    repository: &Path,
    target: &str,
) -> BenchResult<String> {
    let output = Command::new(cli)
        .args([
            "--database",
            path_text(database)?,
            "--worker",
            path_text(worker)?,
            "why",
            target,
            "--json",
        ])
        .current_dir(repository)
        .output()?;
    if !output.status.success() {
        return Err(failure(format!(
            "warm CLI query failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}

async fn benchmark_mcp(
    cli: &Path,
    worker: &Path,
    database: &Path,
    repository: &Path,
    target: &str,
    samples: usize,
) -> BenchResult<(Distribution, bool)> {
    let transport =
        TokioChildProcess::new(tokio::process::Command::new(cli).configure(|command| {
            command.current_dir(repository).args([
                "--database",
                path_text(database).expect("validated database path"),
                "--worker",
                path_text(worker).expect("validated worker path"),
                "serve",
            ]);
        }))?;
    let client = ().serve(transport).await?;
    call_mcp(&client, target).await?;
    let mut durations = Vec::with_capacity(samples);
    let mut payloads = BTreeSet::new();
    for _ in 0..samples {
        let started = Instant::now();
        let payload = call_mcp(&client, target).await?;
        durations.push(started.elapsed());
        payloads.insert(payload);
    }
    client.cancel().await?;
    Ok((
        Distribution::from_durations(&durations),
        payloads.len() == 1,
    ))
}

async fn call_mcp(
    client: &rmcp::service::RunningService<rmcp::RoleClient, ()>,
    target: &str,
) -> BenchResult<String> {
    let Value::Object(arguments) = serde_json::json!({ "target": target }) else {
        unreachable!("tool arguments are an object")
    };
    let result = client
        .call_tool(CallToolRequestParams::new("explain_rationale").with_arguments(arguments))
        .await?;
    if result.is_error == Some(true) {
        return Err(failure("warm MCP query returned a tool error"));
    }
    let structured = result
        .structured_content
        .ok_or_else(|| failure("warm MCP query omitted structured content"))?;
    Ok(to_canonical_json(&structured)?)
}

async fn memory_stability(
    engine: &Engine,
    target: &str,
    supervisor: &WorkerSupervisor,
    request: &KernelRequest,
    iterations: usize,
) -> BenchResult<(u64, u64)> {
    for _ in 0..10 {
        engine.why(target).await?;
        supervisor.evaluate(request.clone()).await?;
    }
    let before = resident_tree_kib(std::process::id())?;
    for _ in 0..iterations {
        engine.why(target).await?;
        supervisor.evaluate(request.clone()).await?;
    }
    let after = resident_tree_kib(std::process::id())?;
    Ok((before, after))
}

fn fixture_request(root: &Path) -> BenchResult<KernelRequest> {
    let source = fs::read_to_string(root.join("fixtures/protocol/request-established.json"))?;
    Ok(serde_json::from_str(&source)?)
}

fn repository_root() -> BenchResult<PathBuf> {
    Ok(PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?)
}

fn benchmark_executables(
    root: &Path,
    arguments: &Arguments,
) -> BenchResult<(PathBuf, PathBuf, PathBuf)> {
    let paths = (
        resolve(root, &arguments.cli),
        resolve(root, &arguments.worker),
        resolve(root, &arguments.ocaml_bench),
    );
    ensure_executable(&paths.0)?;
    ensure_executable(&paths.1)?;
    ensure_executable(&paths.2)?;
    Ok(paths)
}

fn resolve(root: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    }
}

fn ensure_executable(path: &Path) -> BenchResult<()> {
    if path.is_file() {
        Ok(())
    } else {
        Err(failure(format!(
            "required benchmark executable is missing: {}",
            path.display()
        )))
    }
}

fn path_text(path: &Path) -> BenchResult<&str> {
    path.to_str()
        .ok_or_else(|| failure(format!("path is not UTF-8: {}", path.display())))
}

fn failure(message: impl Into<String>) -> BenchError {
    std::io::Error::other(message.into()).into()
}
