//! Cross-language integration tests for the supervised OCaml proof worker.

use std::{env, fs, path::PathBuf, process::Command, time::Duration};

use rationale_model::{Conflict, EdgeKind, KernelRequest, KernelResponse, NodeKind, Verdict};
use rationale_protocol::{
    ProofUnavailableReason, WorkerConfig, WorkerSupervisor, to_canonical_json,
};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/protocol")
        .join(name)
}

#[cfg(unix)]
fn fixture_worker(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn request() -> KernelRequest {
    let source = fs::read_to_string(fixture("request-established.json"))
        .expect("request fixture should be readable");
    serde_json::from_str(&source).expect("request fixture should match Rust types")
}

fn worker_path() -> Option<PathBuf> {
    env::var_os("RATIONALE_KERNEL_WORKER")
        .map(PathBuf::from)
        .filter(|path| path.is_file())
}

async fn supervisor() -> Option<WorkerSupervisor> {
    let path = worker_path()?;
    Some(
        WorkerSupervisor::start(WorkerConfig::new(path))
            .await
            .expect("real OCaml worker should start"),
    )
}

fn expected(name: &str) -> String {
    fs::read_to_string(fixture(&format!("response-{name}.canonical.json")))
        .expect("response fixture should be readable")
        .trim_end()
        .to_owned()
}

fn partial_request() -> KernelRequest {
    let mut request = request();
    "request-partial-1".clone_into(&mut request.request_id);
    request.goal.anchor_kinds = vec![NodeKind::Decision];
    request
        .edges
        .retain(|edge| edge.kind != EdgeKind::Documents);
    request
}

fn not_established_request() -> KernelRequest {
    let mut request = request();
    "request-not-established-1".clone_into(&mut request.request_id);
    request.goal.anchor_kinds = vec![NodeKind::Decision];
    request.edges.clear();
    request
}

fn conflicted_request() -> KernelRequest {
    let mut request = request();
    "request-conflicted-1".clone_into(&mut request.request_id);
    request.goal.anchor_kinds = vec![NodeKind::Decision];

    let template = request
        .nodes
        .iter()
        .find(|node| node.kind == NodeKind::Decision)
        .expect("fixture should have a decision")
        .clone();
    request.nodes.retain(|node| node.kind != NodeKind::Decision);
    let mut local = template.clone();
    "decision-local".clone_into(&mut local.id);
    local.subject_id = None;
    local.outcome_id = None;
    let mut remote = local.clone();
    "decision-remote".clone_into(&mut remote.id);
    request.nodes.extend([local, remote]);

    let template = request
        .edges
        .iter()
        .find(|edge| edge.kind == EdgeKind::Documents)
        .expect("fixture should have a documents edge")
        .clone();
    request.edges.retain(|edge| {
        edge.kind != EdgeKind::Documents
            && edge.source_id != "decision-1"
            && edge.target_id != "decision-1"
    });
    let mut local_edge = template.clone();
    "edge-documents-local".clone_into(&mut local_edge.id);
    "decision-local".clone_into(&mut local_edge.target_id);
    let mut remote_edge = template;
    "edge-documents-remote".clone_into(&mut remote_edge.id);
    "decision-remote".clone_into(&mut remote_edge.target_id);
    request.edges.extend([local_edge, remote_edge]);
    request.conflicts = vec![Conflict {
        left_node_id: "decision-local".to_owned(),
        right_node_id: "decision-remote".to_owned(),
        rule_id: "conflict.explicit".to_owned(),
    }];
    request
}

#[tokio::test]
async fn real_worker_matches_every_golden_verdict() {
    let Some(supervisor) = supervisor().await else {
        eprintln!("skipping cross-language test: RATIONALE_KERNEL_WORKER is unset");
        return;
    };
    for (name, request) in [
        ("established", request()),
        ("partial", partial_request()),
        ("not-established", not_established_request()),
        ("conflicted", conflicted_request()),
    ] {
        let response = supervisor
            .evaluate(request)
            .await
            .expect("OCaml evaluation should succeed");
        assert_eq!(
            to_canonical_json(&response).expect("response should encode"),
            expected(name),
            "golden response changed for {name}"
        );
    }
}

#[tokio::test]
async fn removing_each_decisive_edge_downgrades_the_complete_proof() {
    let Some(supervisor) = supervisor().await else {
        eprintln!("skipping cross-language test: RATIONALE_KERNEL_WORKER is unset");
        return;
    };
    let mut complete = request();
    complete.goal.anchor_kinds = vec![NodeKind::Decision];
    let response = supervisor
        .evaluate(complete.clone())
        .await
        .expect("complete proof should evaluate");
    assert_eq!(proof_verdict(&response), Verdict::Established);

    for (kind, expected) in [
        (EdgeKind::IntroducedBy, Verdict::NotEstablished),
        (EdgeKind::IncludedInPr, Verdict::Partial),
        (EdgeKind::ResolvesIssue, Verdict::Partial),
        (EdgeKind::Documents, Verdict::Partial),
    ] {
        let mut mutated = complete.clone();
        mutated.request_id = format!("mutation-{kind:?}");
        mutated.edges.retain(|edge| edge.kind != kind);
        let response = supervisor
            .evaluate(mutated)
            .await
            .expect("mutated proof should evaluate");
        assert_eq!(proof_verdict(&response), expected, "removed {kind:?}");
    }
}

fn proof_verdict(response: &KernelResponse) -> Verdict {
    let KernelResponse::Proof { proof, .. } = response else {
        panic!("kernel should return a proof response");
    };
    proof.verdict
}

#[tokio::test]
async fn oversized_request_is_unavailable_not_a_verdict() {
    let Some(path) = worker_path() else {
        eprintln!("skipping cross-language test: RATIONALE_KERNEL_WORKER is unset");
        return;
    };
    let supervisor = WorkerSupervisor::start(
        WorkerConfig::new(path)
            .with_max_payload(64)
            .with_restart_policy(0, Duration::ZERO),
    )
    .await
    .expect("handshake should fit within the test frame limit");
    let error = supervisor
        .evaluate(request())
        .await
        .expect_err("oversized request must not produce a verdict");
    assert!(matches!(
        error.reason,
        ProofUnavailableReason::FrameTooLarge { .. }
    ));
}

#[cfg(unix)]
#[tokio::test]
async fn wrong_worker_version_fails_handshake() {
    let error =
        WorkerSupervisor::start(WorkerConfig::new(fixture_worker("wrong-version-worker.sh")))
            .await
            .expect_err("a mismatched worker must not start");
    assert!(matches!(
        error.reason,
        ProofUnavailableReason::HandshakeFailed { .. }
    ));
}

#[cfg(unix)]
#[tokio::test]
async fn malformed_worker_response_is_unavailable() {
    let supervisor = WorkerSupervisor::start(
        WorkerConfig::new(fixture_worker("malformed-response-worker.sh"))
            .with_restart_policy(0, Duration::ZERO),
    )
    .await
    .expect("fixture worker should complete its handshake");
    let error = supervisor
        .evaluate(request())
        .await
        .expect_err("malformed JSON must not produce a verdict");
    assert!(matches!(
        error.reason,
        ProofUnavailableReason::ProtocolViolation { .. }
    ));
}

#[cfg(unix)]
#[tokio::test]
async fn oversized_worker_response_is_unavailable() {
    let supervisor = WorkerSupervisor::start(
        WorkerConfig::new(fixture_worker("oversized-response-worker.sh"))
            .with_max_payload(4096)
            .with_restart_policy(0, Duration::ZERO),
    )
    .await
    .expect("fixture worker should complete its handshake");
    let error = supervisor
        .evaluate(request())
        .await
        .expect_err("oversized response must not produce a verdict");
    assert!(
        matches!(
            error.reason,
            ProofUnavailableReason::FrameTooLarge {
                length: 8192,
                limit: 4096
            }
        ),
        "unexpected error: {:?}",
        error.reason
    );
}

#[cfg(unix)]
#[tokio::test]
async fn hanging_worker_times_out_without_fallback() {
    let supervisor = WorkerSupervisor::start(
        WorkerConfig::new(fixture_worker("hanging-worker.sh"))
            .with_request_timeout(Duration::from_millis(500))
            .with_restart_policy(0, Duration::ZERO),
    )
    .await
    .expect("fixture worker should complete its handshake");
    let error = supervisor
        .evaluate(request())
        .await
        .expect_err("a hung worker must not produce a fallback verdict");
    assert_eq!(error.reason, ProofUnavailableReason::TimedOut);
}

#[cfg(unix)]
fn signal(process_id: u32, signal: &str) {
    let status = Command::new("kill")
        .args([signal, &process_id.to_string()])
        .status()
        .expect("kill command should run");
    assert!(status.success(), "kill {signal} should succeed");
}

#[cfg(unix)]
#[tokio::test]
async fn caller_cancellation_cannot_desynchronise_worker() {
    let Some(supervisor) = supervisor().await else {
        eprintln!("skipping cross-language test: RATIONALE_KERNEL_WORKER is unset");
        return;
    };
    let process_id = supervisor
        .worker_process_id()
        .expect("worker should expose its process id");
    signal(process_id, "-STOP");

    let cancelled_supervisor = supervisor.clone();
    let cancelled = tokio::spawn(async move { cancelled_supervisor.evaluate(request()).await });
    tokio::time::sleep(Duration::from_millis(20)).await;
    cancelled.abort();
    signal(process_id, "-CONT");

    let response = supervisor
        .evaluate(request())
        .await
        .expect("next request should remain synchronized");
    assert_eq!(
        to_canonical_json(&response).expect("response should encode"),
        expected("established")
    );
}

#[cfg(unix)]
#[tokio::test]
async fn crash_mid_request_is_unavailable_without_fallback() {
    let Some(path) = worker_path() else {
        eprintln!("skipping cross-language test: RATIONALE_KERNEL_WORKER is unset");
        return;
    };
    let supervisor = WorkerSupervisor::start(
        WorkerConfig::new(path)
            .with_request_timeout(Duration::from_secs(1))
            .with_restart_policy(0, Duration::ZERO),
    )
    .await
    .expect("worker should start");
    let process_id = supervisor
        .worker_process_id()
        .expect("worker should expose its process id");
    signal(process_id, "-STOP");

    let evaluating = tokio::spawn(async move { supervisor.evaluate(request()).await });
    tokio::time::sleep(Duration::from_millis(20)).await;
    signal(process_id, "-KILL");
    let error = evaluating
        .await
        .expect("evaluation task should complete")
        .expect_err("a killed worker must not produce a fallback verdict");
    assert!(matches!(
        error.reason,
        ProofUnavailableReason::Crashed { .. }
    ));
}

#[cfg(unix)]
#[tokio::test]
async fn crashed_worker_restarts_within_bound() {
    let Some(supervisor) = supervisor().await else {
        eprintln!("skipping cross-language test: RATIONALE_KERNEL_WORKER is unset");
        return;
    };
    let original_process_id = supervisor
        .worker_process_id()
        .expect("worker should expose its process id");
    signal(original_process_id, "-KILL");

    let response = supervisor
        .evaluate(request())
        .await
        .expect("supervisor should restart the pure worker once");
    assert_eq!(
        to_canonical_json(&response).expect("response should encode"),
        expected("established")
    );
    assert_ne!(
        supervisor.worker_process_id(),
        Some(original_process_id),
        "restart should publish the replacement process id"
    );
}
