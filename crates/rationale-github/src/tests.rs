use std::{
    fmt::Write as _,
    fs,
    io::{Read, Write as _},
    net::TcpListener,
    path::PathBuf,
    thread,
};

use rationale_model::{EdgeKind, NodeKind};

use super::{
    GitHubClient, GitHubCursor, GitHubError, GitHubRepository, GitHubSyncOutcome,
    normalize::closing_references,
};

struct StubResponse {
    status: &'static str,
    headers: Vec<(String, String)>,
    body: String,
}

fn fixture(name: &str) -> String {
    fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/github")
            .join(name),
    )
    .expect("GitHub fixture should be readable")
}

fn stub_server(
    build_responses: impl FnOnce(std::net::SocketAddr) -> Vec<StubResponse>,
) -> (String, thread::JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("fixture server should bind");
    let address = listener
        .local_addr()
        .expect("fixture server should have an address");
    let responses = build_responses(address);
    let handle = thread::spawn(move || {
        let mut requests = Vec::new();
        for response in responses {
            let (mut stream, _) = listener.accept().expect("fixture request should arrive");
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
            requests.push(String::from_utf8(request).expect("request should be UTF-8"));
            let mut headers = format!(
                "HTTP/1.1 {}\r\nContent-Length: {}\r\nConnection: close\r\n",
                response.status,
                response.body.len()
            );
            for (name, value) in response.headers {
                write!(headers, "{name}: {value}\r\n")
                    .expect("response header formatting should succeed");
            }
            headers.push_str("\r\n");
            stream
                .write_all(headers.as_bytes())
                .expect("response headers should be written");
            stream
                .write_all(response.body.as_bytes())
                .expect("response body should be written");
        }
        requests
    });
    (format!("http://{address}/"), handle)
}

#[test]
fn normalizes_supported_remote_forms() {
    for remote in [
        "git@github.com:Acme/Widget.git",
        "ssh://git@github.com/Acme/Widget.git",
        "https://github.com/Acme/Widget.git",
    ] {
        let repository =
            GitHubRepository::from_remote(remote).expect("GitHub remote should normalize");
        assert_eq!(repository.slug(), "acme/widget");
    }
}

#[test]
fn rejects_non_github_ambiguous_and_credentialed_remotes() {
    for remote in [
        "https://gitlab.com/acme/widget.git",
        "https://token@github.com/acme/widget.git",
        "https://github.com/acme/widget/extra",
        "git@github.com:../widget.git",
    ] {
        assert!(GitHubRepository::from_remote(remote).is_err());
    }
}

#[test]
fn rejects_credentialed_or_ambiguous_api_bases() {
    for base in [
        "https://token@api.github.com/",
        "https://api.github.com/?token=secret",
        "file:///tmp/github",
    ] {
        assert!(GitHubClient::with_api_base(base, None).is_err());
    }
}

#[test]
fn recognizes_only_adjacent_same_repository_closing_syntax() {
    let repository = GitHubRepository::from_remote("git@github.com:acme/widget.git")
        .expect("remote should normalize");
    assert_eq!(
        closing_references(
            "Fixes #1, resolves acme/widget#2 and fixed \
             https://github.com/acme/widget/issues/3. Mentions #9; fixes other/repo#4.",
            &repository,
        ),
        [1, 2, 3].into_iter().collect()
    );
}

#[tokio::test]
async fn paginates_conditionally_and_normalizes_explicit_edges() {
    let (base, handle) = stub_server(|address| {
        let next = format!(
            "http://{address}/repos/acme/widget/issues?state=all&sort=updated&direction=desc&per_page=100&page=2"
        );
        vec![
            StubResponse {
                status: "200 OK",
                headers: vec![
                    ("ETag".to_owned(), "\"issues-v2\"".to_owned()),
                    ("Link".to_owned(), format!("<{next}>; rel=\"next\"")),
                    ("X-RateLimit-Limit".to_owned(), "5000".to_owned()),
                    ("X-RateLimit-Remaining".to_owned(), "4999".to_owned()),
                    ("X-RateLimit-Reset".to_owned(), "1789150000".to_owned()),
                ],
                body: fixture("issues-page-1.json"),
            },
            StubResponse {
                status: "200 OK",
                headers: vec![("X-RateLimit-Remaining".to_owned(), "4998".to_owned())],
                body: fixture("issues-page-2.json"),
            },
            StubResponse {
                status: "200 OK",
                headers: vec![("X-RateLimit-Remaining".to_owned(), "4997".to_owned())],
                body: fixture("pull-7-commits-page-1.json"),
            },
        ]
    });
    let client = GitHubClient::with_api_base(&base, Some("top-secret"))
        .expect("fixture client should build");
    let repository = GitHubRepository::from_remote("https://github.com/acme/widget.git")
        .expect("remote should normalize");
    let cursor = GitHubCursor {
        issues_etag: Some("\"issues-v1\"".to_owned()),
    }
    .encode()
    .expect("cursor should encode");
    let result = client
        .synchronize(&repository, Some(&cursor))
        .await
        .expect("fixture sync should succeed");
    let GitHubSyncOutcome::Updated(evidence) = result else {
        panic!("fixture should be updated");
    };
    assert_eq!(evidence.records.len(), 3);
    assert_eq!(evidence.edges.len(), 2);
    let issue = evidence
        .records
        .iter()
        .find(|record| record.id == "#42")
        .expect("issue should be normalized");
    assert_eq!(issue.kind, NodeKind::WorkItem);
    assert_eq!(issue.subject_id, None);
    assert_eq!(issue.outcome_id, None);
    assert_eq!(issue.origin.observed_at, None);
    assert!(evidence.edges.iter().any(|edge| {
        edge.kind == EdgeKind::IncludedInPr
            && edge.target_id == "PR-7"
            && edge.source_id == "1111111111111111111111111111111111111111"
    }));
    assert!(evidence.edges.iter().any(|edge| {
        edge.kind == EdgeKind::ResolvesIssue && edge.source_id == "PR-7" && edge.target_id == "#42"
    }));
    assert_eq!(
        evidence.cursor.issues_etag.as_deref(),
        Some("\"issues-v2\"")
    );
    assert_eq!(evidence.rate_limit.remaining, Some(4_997));

    let requests = handle.join().expect("fixture server should finish");
    assert_eq!(requests.len(), 3);
    let first = requests[0].to_ascii_lowercase();
    assert!(first.contains("if-none-match: \"issues-v1\""));
    assert!(first.contains("authorization: bearer top-secret"));
    assert!(first.contains("x-github-api-version: 2026-03-10"));
    assert!(!format!("{client:?}").contains("top-secret"));
    assert!(!cursor.contains("top-secret"));
}

#[tokio::test]
async fn not_modified_preserves_the_caller_cursor() {
    let responses = vec![StubResponse {
        status: "304 Not Modified",
        headers: vec![("X-RateLimit-Remaining".to_owned(), "4999".to_owned())],
        body: String::new(),
    }];
    let (base, handle) = stub_server(|_| responses);
    let client = GitHubClient::with_api_base(&base, None).expect("fixture client should build");
    let repository = GitHubRepository::from_remote("https://github.com/acme/widget.git")
        .expect("remote should normalize");
    let cursor = GitHubCursor {
        issues_etag: Some("\"current\"".to_owned()),
    };
    let encoded = cursor.encode().expect("cursor should encode");
    let outcome = client
        .synchronize(&repository, Some(&encoded))
        .await
        .expect("304 should be handled");
    assert_eq!(
        outcome,
        GitHubSyncOutcome::NotModified {
            cursor,
            rate_limit: super::RateLimit {
                limit: None,
                remaining: Some(4_999),
                reset_at: None,
            },
        }
    );
    assert_eq!(
        handle.join().expect("fixture server should finish").len(),
        1
    );
}

#[tokio::test]
async fn rejects_cross_origin_pagination() {
    let responses = vec![StubResponse {
        status: "200 OK",
        headers: vec![(
            "Link".to_owned(),
            "<https://attacker.invalid/repos/acme/widget/issues?page=2>; rel=\"next\"".to_owned(),
        )],
        body: fixture("issues-page-1.json"),
    }];
    let (base, handle) = stub_server(|_| responses);
    let client = GitHubClient::with_api_base(&base, None).expect("fixture client should build");
    let repository = GitHubRepository::from_remote("https://github.com/acme/widget.git")
        .expect("remote should normalize");
    let error = client
        .synchronize(&repository, None)
        .await
        .expect_err("cross-origin links must fail closed");
    assert!(matches!(error, GitHubError::InvalidPagination));
    assert_eq!(
        handle.join().expect("fixture server should finish").len(),
        1
    );
}

#[tokio::test]
async fn status_errors_never_include_tokens_or_response_bodies() {
    let responses = vec![StubResponse {
        status: "401 Unauthorized",
        headers: Vec::new(),
        body: r#"{"message":"top-secret response"}"#.to_owned(),
    }];
    let (base, handle) = stub_server(|_| responses);
    let client = GitHubClient::with_api_base(&base, Some("top-secret"))
        .expect("fixture client should build");
    let repository = GitHubRepository::from_remote("https://github.com/acme/widget.git")
        .expect("remote should normalize");
    let error = client
        .synchronize(&repository, None)
        .await
        .expect_err("status should fail");
    let rendered = format!("{error:?} {error}");
    assert!(!rendered.contains("top-secret"));
    assert!(!rendered.contains("response"));
    assert_eq!(
        handle.join().expect("fixture server should finish").len(),
        1
    );
}
