//! End-to-end coverage for combined local and GitHub evidence.

use std::{
    env, fs,
    io::{Read, Write},
    net::TcpListener,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
    thread,
};

use rationale_engine::Engine;
use rationale_github::GitHubClient;
use rationale_model::{KernelResponse, Verdict};

static NEXT_REPOSITORY: AtomicU64 = AtomicU64::new(0);

struct TestRepository {
    path: PathBuf,
}

impl TestRepository {
    fn new() -> Self {
        let serial = NEXT_REPOSITORY.fetch_add(1, Ordering::Relaxed);
        let path = env::temp_dir().join(format!(
            "rationale-engine-github-{}-{serial}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("temporary repository should be created");
        let repository = Self { path };
        repository.git(&["init", "--quiet"]);
        repository.git(&["config", "user.name", "Rationale Test"]);
        repository.git(&["config", "user.email", "rationale@example.invalid"]);
        repository.git(&["remote", "add", "origin", "git@github.com:acme/widget.git"]);
        repository
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

    fn commit(&self) -> String {
        fs::create_dir_all(self.path.join("src")).expect("source directory should exist");
        fs::write(
            self.path.join("src/app.txt"),
            "explain the checkout policy\n",
        )
        .expect("fixture should be written");
        self.git(&["add", "."]);
        self.git(&[
            "commit",
            "--quiet",
            "-m",
            "feat(checkout): define the retry policy",
        ]);
        self.git(&["rev-parse", "HEAD"])
    }
}

impl Drop for TestRepository {
    fn drop(&mut self) {
        let _ignored = fs::remove_dir_all(&self.path);
    }
}

fn stub_github(commit: &str) -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("fixture server should bind");
    let address = listener
        .local_addr()
        .expect("fixture server should have an address");
    let bodies = [
        serde_json::json!([
            {
                "number": 7,
                "body": "Fixes #42",
                "html_url": "https://github.com/acme/widget/pull/7",
                "pull_request": { "url": "https://api.github.com/repos/acme/widget/pulls/7" }
            },
            {
                "number": 42,
                "body": "Checkout retry requirements",
                "html_url": "https://github.com/acme/widget/issues/42"
            }
        ])
        .to_string(),
        serde_json::json!([{
            "sha": commit,
            "html_url": format!("https://github.com/acme/widget/commit/{commit}")
        }])
        .to_string(),
    ];
    let handle = thread::spawn(move || {
        for (index, body) in bodies.into_iter().enumerate() {
            let (mut stream, _) = listener.accept().expect("fixture request should arrive");
            read_request(&mut stream);
            let etag = if index == 0 {
                "ETag: \"github-v1\"\r\n"
            } else {
                ""
            };
            let headers = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\nX-RateLimit-Limit: 5000\r\nX-RateLimit-Remaining: {}\r\n{etag}\r\n",
                body.len(),
                4_999 - index
            );
            stream
                .write_all(headers.as_bytes())
                .expect("response headers should be written");
            stream
                .write_all(body.as_bytes())
                .expect("response body should be written");
        }
    });
    (format!("http://{address}/"), handle)
}

fn read_request(stream: &mut impl Read) {
    let mut request = Vec::new();
    let mut buffer = [0_u8; 4_096];
    loop {
        let read = stream
            .read(&mut buffer)
            .expect("request should be readable");
        if read == 0 {
            return;
        }
        request.extend_from_slice(&buffer[..read]);
        if request.windows(4).any(|window| window == b"\r\n\r\n") {
            return;
        }
    }
}

fn worker_path() -> Option<PathBuf> {
    env::var_os("RATIONALE_KERNEL_WORKER")
        .map(PathBuf::from)
        .filter(|path| path.is_file())
}

#[tokio::test]
async fn github_completes_the_proof_and_failed_refresh_preserves_it_as_stale() {
    let repository = TestRepository::new();
    let commit = repository.commit();
    let (base, server) = stub_github(&commit);
    let client = GitHubClient::with_api_base(&base, None).expect("fixture client should build");
    let engine = Engine::discover(
        &repository.path,
        Some(Path::new(".rationale/test.db").into()),
    )
    .expect("engine should discover the repository")
    .with_worker_path(worker_path());

    let first = engine
        .sync_with_github_client(&client)
        .await
        .expect("combined sync should publish");
    server.join().expect("fixture server should finish");
    assert!(first.stale_sources.is_empty());
    assert_eq!(first.records, 3);
    assert_eq!(first.edges, 2);
    assert_eq!(
        first
            .github_rate_limit
            .as_ref()
            .and_then(|rate| rate.remaining),
        Some(4_998)
    );
    assert!(
        engine
            .show("PR-7")
            .expect("record lookup should work")
            .is_some()
    );

    if worker_path().is_some() {
        let proof = engine
            .why("src/app.txt:1")
            .await
            .expect("GitHub relationship should be provable");
        let KernelResponse::Proof { proof, .. } = &proof.response else {
            panic!("kernel should return a proof response");
        };
        assert_eq!(proof.verdict, Verdict::Established);
        assert!(proof.proof_chains.iter().any(|chain| {
            chain
                .node_ids
                .ends_with(&["PR-7".to_owned(), "#42".to_owned()])
        }));
    }

    let unavailable = GitHubClient::with_api_base(&base, None)
        .expect("unavailable fixture client should still configure");
    let stale = engine
        .sync_with_github_client(&unavailable)
        .await
        .expect("failed refresh should retain prior evidence");
    assert_eq!(stale.stale_sources, ["github:acme/widget"]);
    assert_eq!(stale.records, 3);
    assert_eq!(stale.edges, 2);
    assert_eq!(stale.quarantined, 1);

    if worker_path().is_some() {
        let proof = engine
            .why("src/app.txt:1")
            .await
            .expect("retained evidence should remain provable");
        assert!(proof.has_stale_source());
        let KernelResponse::Proof { proof, .. } = proof.response else {
            panic!("kernel should return a proof response");
        };
        assert_eq!(proof.verdict, Verdict::Established);
    }
}
