//! End-to-end coverage for the local CLI and real OCaml proof worker.

use std::{
    collections::BTreeSet,
    env, fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
};

use rmcp::{
    ServiceExt,
    model::CallToolRequestParams,
    transport::{ConfigureCommandExt, TokioChildProcess},
};
use serde_json::Value;

static NEXT_REPOSITORY: AtomicU64 = AtomicU64::new(0);

struct TestRepository {
    path: PathBuf,
}

impl TestRepository {
    fn new() -> Self {
        let serial = NEXT_REPOSITORY.fetch_add(1, Ordering::Relaxed);
        let path = env::temp_dir().join(format!("rationale-cli-{}-{serial}", std::process::id()));
        fs::create_dir_all(&path).expect("temporary repository should be created");
        let repository = Self { path };
        repository.git(&["init", "--quiet"]);
        repository.git(&["config", "user.name", "Rationale Test"]);
        repository.git(&["config", "user.email", "rationale@example.invalid"]);
        repository
    }

    fn write(&self, relative: &str, contents: &str) {
        let path = self.path.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("fixture parent should be created");
        }
        fs::write(path, contents).expect("fixture should be written");
    }

    fn git(&self, arguments: &[&str]) {
        let output = Command::new("git")
            .args(arguments)
            .current_dir(&self.path)
            .output()
            .expect("git should run");
        assert!(
            output.status.success(),
            "git {arguments:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn commit_evidence(&self) {
        self.write("src/app.txt", "explain this line\n");
        self.write(
            "governance/STORY-1.md",
            r"---
rationale:
  id: STORY-1
  kind: work_item
  status: accepted
  documents:
    - ADR-0001
---

# Story 1
",
        );
        self.write(
            "docs/ADR-0001.md",
            r"---
rationale:
  id: ADR-0001
  kind: decision
  status: current
---

# Decision 1
",
        );
        self.git(&["add", "."]);
        self.git(&[
            "commit",
            "--quiet",
            "-m",
            "feat(app): explain the first line\n\nStory: #1 | Lineage: 1.1.0",
        ]);
    }

    fn commit_without_reference(&self) {
        self.write("src/checkout.txt", "retry checkout\n");
        self.git(&["add", "."]);
        self.git(&[
            "commit",
            "--quiet",
            "-m",
            "feat(checkout): define checkout retry policy",
        ]);
    }

    fn rationale(&self, arguments: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_rationale"))
            .args(arguments)
            .current_dir(&self.path)
            .output()
            .expect("rationale should run")
    }
}

impl Drop for TestRepository {
    fn drop(&mut self) {
        let _ignored = fs::remove_dir_all(&self.path);
    }
}

fn worker_path() -> Option<PathBuf> {
    env::var_os("RATIONALE_KERNEL_WORKER")
        .map(PathBuf::from)
        .filter(|path| path.is_file())
}

fn stdout(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).expect("stdout should be UTF-8")
}

fn json(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "stdout should be JSON ({error}): {}",
            String::from_utf8_lossy(&output.stdout)
        )
    })
}

fn assert_exit(output: &Output, expected: i32) {
    assert_eq!(
        output.status.code(),
        Some(expected),
        "unexpected stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn arguments(value: Value) -> serde_json::Map<String, Value> {
    let Value::Object(arguments) = value else {
        panic!("tool arguments should be an object");
    };
    arguments
}

fn assert_read_only_tools(tools: &[rmcp::model::Tool]) {
    let names: BTreeSet<_> = tools.iter().map(|tool| tool.name.as_ref()).collect();
    assert_eq!(
        names,
        BTreeSet::from([
            "explain_rationale",
            "find_rationale_gaps",
            "get_evidence",
            "search_candidate_evidence",
        ])
    );
    for tool in tools {
        assert_eq!(
            tool.input_schema.get("additionalProperties"),
            Some(&Value::Bool(false))
        );
        assert!(tool.output_schema.is_some());
        let annotations = tool.annotations.as_ref().expect("annotations should exist");
        assert_eq!(annotations.read_only_hint, Some(true));
        assert_eq!(annotations.destructive_hint, Some(false));
        assert_eq!(annotations.open_world_hint, Some(false));
    }
}

#[test]
fn generated_repository_supports_all_local_commands() {
    let Some(worker) = worker_path() else {
        eprintln!("skipping cross-language test: RATIONALE_KERNEL_WORKER is unset");
        return;
    };
    let repository = TestRepository::new();
    repository.commit_evidence();
    let database = ".rationale/test.db";

    let sync = repository.rationale(&["--database", database, "sync", "--local", "--json"]);
    assert_exit(&sync, 0);
    let sync_json = json(&sync);
    assert_eq!(sync_json["records"], 3);
    assert_eq!(sync_json["edges"], 2);
    assert_eq!(sync_json["quarantined"], 0);

    let worker = worker.to_string_lossy();
    let why_json = repository.rationale(&[
        "--database",
        database,
        "--worker",
        &worker,
        "why",
        "src/app.txt:1",
        "--json",
    ]);
    assert_exit(&why_json, 0);
    let proof = json(&why_json);
    assert_eq!(proof["response"]["status"], "proof");
    assert_eq!(proof["response"]["proof"]["verdict"], "established");
    assert_eq!(proof["candidates"], serde_json::json!([]));
    assert_eq!(
        proof["response"]["proof"]["proof_chains"][0]["node_ids"][2],
        "STORY-1"
    );
    assert_eq!(
        proof["response"]["proof"]["proof_chains"][0]["node_ids"][3],
        "ADR-0001"
    );

    let why_human = repository.rationale(&[
        "--database",
        database,
        "--worker",
        &worker,
        "why",
        "src/app.txt:1",
    ]);
    assert_exit(&why_human, 0);
    let human = stdout(&why_human);
    let headings = [
        "Verdict:",
        "Proof paths:",
        "Gaps:",
        "Candidates:",
        "Freshness:",
    ];
    let positions: Vec<_> = headings
        .iter()
        .map(|heading| human.find(heading).expect("heading should be rendered"))
        .collect();
    assert!(positions.windows(2).all(|pair| pair[0] < pair[1]));

    let show = repository.rationale(&["--database", database, "show", "STORY-1", "--json"]);
    assert_exit(&show, 0);
    assert_eq!(json(&show)["kind"], "work_item");

    repository.write("src/app.txt", "explain the changed line\n");
    let gaps = repository.rationale(&["--database", database, "gaps", "--changed", "--json"]);
    assert_exit(&gaps, 2);
    assert_eq!(json(&gaps)["gaps"][0]["path"], "src/app.txt");
}

#[test]
fn exit_categories_keep_missing_invalid_and_unavailable_distinct() {
    let repository = TestRepository::new();
    repository.commit_evidence();
    let database = ".rationale/test.db";
    assert_exit(
        &repository.rationale(&["--database", database, "sync", "--local"]),
        0,
    );

    let missing = repository.rationale(&["--database", database, "show", "STORY-404", "--json"]);
    assert_exit(&missing, 2);
    assert_eq!(json(&missing)["status"], "missing");

    let invalid = repository.rationale(&["--database", database, "why", "not-a-target", "--json"]);
    assert_exit(&invalid, 4);
    assert_eq!(json(&invalid)["error"]["category"], "invalid_input");

    let unavailable = repository.rationale(&[
        "--database",
        database,
        "--worker",
        path_string(&repository.path.join("missing-worker")),
        "why",
        "src/app.txt:1",
        "--json",
    ]);
    assert_exit(&unavailable, 6);
    assert_eq!(json(&unavailable)["error"]["category"], "proof_unavailable");
}

#[test]
fn candidate_records_are_ranked_without_changing_the_verdict() {
    let Some(worker) = worker_path() else {
        eprintln!("skipping cross-language test: RATIONALE_KERNEL_WORKER is unset");
        return;
    };
    let repository = TestRepository::new();
    repository.commit_without_reference();
    let database = ".rationale/candidates.db";
    let worker = worker.to_string_lossy();

    assert_exit(
        &repository.rationale(&["--database", database, "sync", "--local"]),
        0,
    );
    let before = repository.rationale(&[
        "--database",
        database,
        "--worker",
        &worker,
        "why",
        "src/checkout.txt:1",
        "--json",
    ]);
    assert_exit(&before, 2);
    let before = json(&before);
    assert_eq!(before["response"]["proof"]["verdict"], "partial");
    assert_eq!(before["candidates"], serde_json::json!([]));

    repository.write(
        "governance/STORY-9.md",
        r"---
rationale:
  id: STORY-9
  kind: work_item
  subject_id: checkout-retry-policy
  status: accepted
---

# Checkout retry policy
",
    );
    assert_exit(
        &repository.rationale(&["--database", database, "sync", "--local"]),
        0,
    );
    let after = repository.rationale(&[
        "--database",
        database,
        "--worker",
        &worker,
        "why",
        "src/checkout.txt:1",
        "--json",
    ]);
    assert_exit(&after, 2);
    let after = json(&after);
    assert_eq!(after["response"]["proof"]["verdict"], "partial");
    assert_eq!(after["candidates"][0]["record_id"], "STORY-9");
    assert_eq!(after["candidates"][0]["score_version"], 1);
    assert!(!after["response"].to_string().contains("STORY-9"));
    let components = &after["candidates"][0]["components"];
    let reconstructed = u64::from(components["exact_identifier"] == true) * 1_000
        + components["token_overlap"]
            .as_u64()
            .expect("token score should be numeric")
            * 100
        + components["path_segment_overlap"]
            .as_u64()
            .expect("path score should be numeric")
            * 20
        + components["source_recency"]
            .as_u64()
            .expect("recency score should be numeric");
    assert_eq!(
        after["candidates"][0]["score"],
        serde_json::json!(reconstructed)
    );
}

#[tokio::test]
async fn mcp_tools_are_read_only_and_match_cli_json() {
    let Some(worker) = worker_path() else {
        eprintln!("skipping cross-language test: RATIONALE_KERNEL_WORKER is unset");
        return;
    };
    let repository = TestRepository::new();
    repository.commit_evidence();
    let database = ".rationale/mcp.db";
    assert_exit(
        &repository.rationale(&["--database", database, "sync", "--local"]),
        0,
    );

    let worker = worker.to_string_lossy().into_owned();
    let transport = TokioChildProcess::new(
        tokio::process::Command::new(env!("CARGO_BIN_EXE_rationale")).configure(|command| {
            command.current_dir(&repository.path).args([
                "--database",
                database,
                "--worker",
                &worker,
                "serve",
            ]);
        }),
    )
    .expect("MCP child process should start");
    let client = ().serve(transport).await.expect("MCP should initialize");

    let tools = client
        .list_all_tools()
        .await
        .expect("tools should be listed");
    assert_read_only_tools(&tools);

    let cli_why = repository.rationale(&[
        "--database",
        database,
        "--worker",
        &worker,
        "why",
        "src/app.txt:1",
        "--json",
    ]);
    assert_exit(&cli_why, 0);
    let mcp_why = client
        .call_tool(
            CallToolRequestParams::new("explain_rationale")
                .with_arguments(arguments(serde_json::json!({ "target": "src/app.txt:1" }))),
        )
        .await
        .expect("explain tool should succeed");
    assert_eq!(mcp_why.structured_content, Some(json(&cli_why)));

    let cli_record = repository.rationale(&["--database", database, "show", "STORY-1", "--json"]);
    assert_exit(&cli_record, 0);
    let mcp_record = client
        .call_tool(
            CallToolRequestParams::new("get_evidence")
                .with_arguments(arguments(serde_json::json!({ "record_id": "STORY-1" }))),
        )
        .await
        .expect("evidence tool should succeed");
    assert_eq!(mcp_record.structured_content, Some(json(&cli_record)));

    let cli_gaps = repository.rationale(&["--database", database, "gaps", "--changed", "--json"]);
    assert_exit(&cli_gaps, 0);
    let mcp_gaps = client
        .call_tool(
            CallToolRequestParams::new("find_rationale_gaps")
                .with_arguments(arguments(serde_json::json!({ "scope": "changed" }))),
        )
        .await
        .expect("gap tool should succeed");
    assert_eq!(mcp_gaps.structured_content, Some(json(&cli_gaps)));

    let search = client
        .call_tool(
            CallToolRequestParams::new("search_candidate_evidence")
                .with_arguments(arguments(serde_json::json!({ "query": "story 1" }))),
        )
        .await
        .expect("candidate tool should succeed");
    let search = search
        .structured_content
        .expect("candidate tool should return structured content");
    assert_eq!(search["query"], "story 1");
    assert_eq!(search["candidates"][0]["record_id"], "STORY-1");

    client.cancel().await.expect("MCP client should shut down");
}

fn path_string(path: &Path) -> &str {
    path.to_str().expect("fixture path should be UTF-8")
}
