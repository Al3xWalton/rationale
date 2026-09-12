//! Public, synthetic CLI and MCP snapshots for all four proof verdicts.

use std::{
    collections::BTreeMap,
    env,
    fmt::Write as _,
    fs,
    io::{Read, Write as _},
    net::TcpListener,
    path::PathBuf,
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
    thread,
};

use rmcp::{
    ServiceExt,
    model::CallToolRequestParams,
    transport::{ConfigureCommandExt, TokioChildProcess},
};
use serde_json::Value;

static NEXT_DEMO: AtomicU64 = AtomicU64::new(0);

struct DemoRepository {
    parent: PathBuf,
    path: PathBuf,
}

impl DemoRepository {
    fn create() -> Self {
        let serial = NEXT_DEMO.fetch_add(1, Ordering::Relaxed);
        let parent = env::temp_dir().join(format!(
            "rationale-public-demo-test-{}-{serial}",
            std::process::id()
        ));
        let path = parent.join("repository");
        fs::create_dir_all(&parent).expect("temporary parent should be created");
        let output = Command::new(project_root().join("scripts/create-demo-repo.sh"))
            .arg(&path)
            .output()
            .expect("demo generator should run");
        assert!(
            output.status.success(),
            "demo generation failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        Self { parent, path }
    }

    fn git(&self, arguments: &[&str]) -> String {
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
        String::from_utf8(output.stdout)
            .expect("git output should be UTF-8")
            .trim()
            .to_owned()
    }

    fn fixture(&self, name: &str) -> String {
        fs::read_to_string(self.path.join(".git/rationale-demo-fixtures").join(name))
            .expect("generated GitHub fixture should be readable")
    }

    fn rationale(&self, arguments: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_rationale"))
            .args(arguments)
            .current_dir(&self.path)
            .output()
            .expect("rationale should run")
    }
}

impl Drop for DemoRepository {
    fn drop(&mut self) {
        let _ignored = fs::remove_dir_all(&self.parent);
    }
}

struct StubResponse {
    status: &'static str,
    headers: Vec<(String, String)>,
    body: String,
}

fn stub_github(repository: &DemoRepository) -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("fixture server should bind");
    let address = listener
        .local_addr()
        .expect("fixture server should have an address");
    let responses = [
        StubResponse {
            status: "200 OK",
            headers: vec![("ETag".to_owned(), "\"demo-v1\"".to_owned())],
            body: repository.fixture("issues.json"),
        },
        StubResponse {
            status: "200 OK",
            headers: Vec::new(),
            body: repository.fixture("pull-11-commits.json"),
        },
        StubResponse {
            status: "200 OK",
            headers: Vec::new(),
            body: repository.fixture("pull-22-commits.json"),
        },
        StubResponse {
            status: "304 Not Modified",
            headers: Vec::new(),
            body: String::new(),
        },
    ];
    let handle = thread::spawn(move || {
        for response in responses {
            let (mut stream, _) = listener.accept().expect("fixture request should arrive");
            let request = read_request(&mut stream);
            if response.status.starts_with("304") {
                assert!(
                    request
                        .to_ascii_lowercase()
                        .contains("if-none-match: \"demo-v1\""),
                    "conditional request should retain the fixture cursor"
                );
            }
            let mut headers = format!(
                "HTTP/1.1 {}\r\nContent-Length: {}\r\nConnection: close\r\nX-RateLimit-Limit: 5000\r\nX-RateLimit-Remaining: 4997\r\n",
                response.status,
                response.body.len()
            );
            for (name, value) in response.headers {
                write!(headers, "{name}: {value}\r\n").expect("header formatting should succeed");
            }
            headers.push_str("\r\n");
            stream
                .write_all(headers.as_bytes())
                .expect("response headers should be written");
            stream
                .write_all(response.body.as_bytes())
                .expect("response body should be written");
        }
    });
    (format!("http://{address}/"), handle)
}

fn read_request(stream: &mut impl Read) -> String {
    let mut request = Vec::new();
    let mut buffer = [0_u8; 4_096];
    loop {
        let read = stream
            .read(&mut buffer)
            .expect("request should be readable");
        if read == 0 {
            break;
        }
        request.extend_from_slice(&buffer[..read]);
        if request.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
    }
    String::from_utf8(request).expect("fixture request should be UTF-8")
}

fn project_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn worker_path() -> Option<PathBuf> {
    env::var_os("RATIONALE_KERNEL_WORKER")
        .map(PathBuf::from)
        .filter(|path| path.is_file())
}

fn assert_exit(output: &Output, expected: i32) {
    assert_eq!(
        output.status.code(),
        Some(expected),
        "unexpected stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn json(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "stdout should be JSON ({error}): {}",
            String::from_utf8_lossy(&output.stdout)
        )
    })
}

fn why(repository: &DemoRepository, worker: &str, target: &str, exit: i32) -> Value {
    let output = repository.rationale(&[
        "--database",
        ".rationale/demo.db",
        "--worker",
        worker,
        "why",
        target,
        "--json",
    ]);
    assert_exit(&output, exit);
    json(&output)
}

fn arguments(value: Value) -> serde_json::Map<String, Value> {
    let Value::Object(arguments) = value else {
        panic!("tool arguments should be an object");
    };
    arguments
}

#[test]
fn generated_history_is_byte_stable_across_directories() {
    let first = DemoRepository::create();
    let second = DemoRepository::create();
    assert_eq!(
        first.git(&["log", "--format=%H %aI %s", "--reverse"]),
        second.git(&["log", "--format=%H %aI %s", "--reverse"])
    );
    assert_eq!(first.fixture("refs.env"), second.fixture("refs.env"));
}

#[tokio::test]
async fn cli_and_mcp_snapshot_all_verdicts_and_supersession() {
    let Some(worker) = worker_path() else {
        eprintln!("skipping cross-language test: RATIONALE_KERNEL_WORKER is unset");
        return;
    };
    let repository = DemoRepository::create();
    repository.git(&["checkout", "--quiet", "demo-conflict"]);
    let (api_base, server) = stub_github(&repository);
    let sync = repository.rationale(&[
        "--database",
        ".rationale/demo.db",
        "--github-api-base",
        &api_base,
        "sync",
        "--json",
    ]);
    assert_exit(&sync, 0);
    assert_eq!(json(&sync)["records"], 13);

    let worker = worker.to_string_lossy().into_owned();
    let targets = BTreeMap::from([
        ("src/complete.rs:2", ("established", 0)),
        ("src/incomplete.rs:2", ("partial", 2)),
        ("commit:demo-absent", ("not_established", 2)),
        ("src/conflict.rs:2", ("conflicted", 3)),
    ]);
    let mut cli_results = BTreeMap::new();
    for (target, (verdict, exit)) in &targets {
        let result = why(&repository, &worker, target, *exit);
        assert_eq!(result["response"]["proof"]["verdict"], *verdict);
        cli_results.insert(*target, result);
    }
    assert_eq!(
        cli_results["src/incomplete.rs:2"]["candidates"][0]["record_id"],
        "STORY-202"
    );
    let complete_chain =
        cli_results["src/complete.rs:2"]["response"]["proof"]["proof_chains"][0]["node_ids"]
            .as_array()
            .expect("complete proof chain should be an array");
    assert_eq!(
        complete_chain[1],
        repository.git(&["rev-parse", "demo-complete"])
    );
    assert_eq!(complete_chain[2..], ["PR-11", "#101", "ADR-0101"]);

    assert_mcp_snapshots(&repository, &worker, &cli_results).await;

    repository.git(&["checkout", "--quiet", "demo-resolved"]);
    let resync = repository.rationale(&[
        "--database",
        ".rationale/demo.db",
        "--github-api-base",
        &api_base,
        "sync",
        "--json",
    ]);
    assert_exit(&resync, 0);
    assert_eq!(json(&resync)["quarantined"], 0);
    server.join().expect("fixture server should finish");

    let resolved = why(&repository, &worker, "src/conflict.rs:2", 0);
    assert_eq!(resolved["response"]["proof"]["verdict"], "established");
    assert_eq!(
        resolved["response"]["proof"]["conflicts"],
        serde_json::json!([])
    );
}

async fn assert_mcp_snapshots(
    repository: &DemoRepository,
    worker: &str,
    expected: &BTreeMap<&str, Value>,
) {
    let transport = TokioChildProcess::new(
        tokio::process::Command::new(env!("CARGO_BIN_EXE_rationale")).configure(|command| {
            command.current_dir(&repository.path).args([
                "--database",
                ".rationale/demo.db",
                "--worker",
                worker,
                "serve",
            ]);
        }),
    )
    .expect("MCP child process should start");
    let client = ().serve(transport).await.expect("MCP should initialize");
    for (target, cli_result) in expected {
        let result = client
            .call_tool(
                CallToolRequestParams::new("explain_rationale")
                    .with_arguments(arguments(serde_json::json!({ "target": target }))),
            )
            .await
            .expect("explain tool should succeed");
        assert_eq!(result.structured_content.as_ref(), Some(cli_result));
    }
    client.cancel().await.expect("MCP client should shut down");
}
