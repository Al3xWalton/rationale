//! Versioned types exchanged by the Rust host and OCaml proof kernel.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// First version of the Rust-to-OCaml proof protocol.
pub const PROTOCOL_VERSION: u16 = 1;

/// Kind of normalized evidence record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    /// A file, symbol, or line range at a repository revision.
    CodeTarget,
    /// A Git commit or normalized change set.
    Change,
    /// An issue, Story, task, or requirement.
    WorkItem,
    /// A pull request or other explicit review artifact.
    Review,
    /// An architecture or design decision.
    Decision,
    /// A test report or explicit verification record.
    Verification,
}

/// Explicit relationship between evidence records.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EdgeKind {
    /// Blame or history establishes which change introduced the target.
    IntroducedBy,
    /// History establishes which change modified the target.
    ChangedBy,
    /// Forge metadata establishes commit membership in a review.
    IncludedInPr,
    /// Native forge metadata or explicit syntax establishes issue resolution.
    ResolvesIssue,
    /// An explicit identifier or link connects documentation to a record.
    Documents,
    /// An explicit declaration makes another record historical.
    Supersedes,
    /// A manifest explicitly connects a verification to its subject.
    VerifiedBy,
}

/// Whether an evidence item participates in a current or historical proof.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RecordStatus {
    /// Evidence is active in the queried snapshot.
    Current,
    /// Evidence is retained for historical inspection only.
    Historical,
}

/// Origin system for an evidence record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    /// Local Git data.
    Git,
    /// Synchronized GitHub data.
    #[serde(rename = "github")]
    GitHub,
    /// A repository document.
    Document,
    /// A verification report or manifest.
    Verification,
    /// A deterministic public test fixture.
    Synthetic,
}

/// Stable citation back to the source of a record or relationship.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Origin {
    /// Source system that supplied the evidence.
    pub source_kind: SourceKind,
    /// Repository-relative path, Git object, or forge resource locator.
    pub locator: String,
    /// Optional immutable source revision.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
    /// Optional timestamp copied from the source snapshot.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observed_at: Option<String>,
}

/// A normalized evidence node supplied to the proof kernel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EvidenceNode {
    /// Content-derived record identifier.
    pub id: String,
    /// Evidence category.
    pub kind: NodeKind,
    /// Current or historical status.
    pub status: RecordStatus,
    /// Optional explicit subject used for structured conflict detection.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject_id: Option<String>,
    /// Optional explicit outcome used for structured conflict detection.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome_id: Option<String>,
    /// Citation to the source artifact.
    pub origin: Origin,
}

/// A normalized admissible relationship supplied to the proof kernel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EvidenceEdge {
    /// Content-derived relationship identifier.
    pub id: String,
    /// Relationship category.
    pub kind: EdgeKind,
    /// Source evidence node identifier.
    pub source_id: String,
    /// Target evidence node identifier.
    pub target_id: String,
    /// Current or historical status.
    pub status: RecordStatus,
    /// Citation to the source that establishes the relationship.
    pub origin: Origin,
}

/// Explicit conflict metadata that can affect the verdict.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Conflict {
    /// First incompatible evidence node.
    pub left_node_id: String,
    /// Second incompatible evidence node.
    pub right_node_id: String,
    /// Stable rule that established the conflict.
    pub rule_id: String,
}

/// Proof objective for one bounded evidence slice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProofGoal {
    /// Node from which proof traversal begins.
    pub target_id: String,
    /// Node categories accepted as recorded-intent anchors.
    pub anchor_kinds: Vec<NodeKind>,
}

/// Request sent to the proof kernel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct KernelRequest {
    /// Protocol version expected by the caller.
    pub protocol_version: u16,
    /// Caller-generated correlation identifier.
    pub request_id: String,
    /// Immutable evidence snapshot identifier.
    pub snapshot_id: String,
    /// Proof objective.
    pub goal: ProofGoal,
    /// Bounded, canonically ordered evidence records.
    pub nodes: Vec<EvidenceNode>,
    /// Bounded, canonically ordered admissible relationships.
    pub edges: Vec<EvidenceEdge>,
    /// Explicit, canonically ordered conflict metadata.
    pub conflicts: Vec<Conflict>,
}

/// Verdict returned by the proof kernel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// A current admissible path reaches a rationale anchor.
    Established,
    /// A path exists but ends at an identifiable missing relationship.
    Partial,
    /// No admissible path establishes the requested rationale.
    NotEstablished,
    /// Current explicit evidence supports incompatible outcomes.
    Conflicted,
}

/// Typed relationship missing from an incomplete proof.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum GapCode {
    /// The code target cannot be connected to a change.
    TargetToChange,
    /// The change cannot be connected to a review.
    ChangeToReview,
    /// The change cannot be connected to a work item.
    ChangeToWorkItem,
    /// The work item cannot be connected to a decision.
    WorkItemToDecision,
    /// A decision lacks explicit verification.
    DecisionToVerification,
    /// No current rationale anchor can be reached.
    NoCurrentAnchor,
}

/// One typed gap in an incomplete proof.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Gap {
    /// Missing relationship category.
    pub code: GapCode,
    /// Last established node before the gap, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_node_id: Option<String>,
    /// Expected destination node category, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to_kind: Option<NodeKind>,
}

/// One machine-checkable admissible route to recorded intent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProofChain {
    /// Ordered evidence node identifiers.
    pub node_ids: Vec<String>,
    /// Ordered relationship identifiers connecting the nodes.
    pub edge_ids: Vec<String>,
    /// Ordered proof rules used for the relationships and conclusion.
    pub rule_ids: Vec<String>,
}

/// Deterministic proof evaluation result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProofResult {
    /// Overall evidence verdict.
    pub verdict: Verdict,
    /// Canonically ordered successful proof routes.
    pub proof_chains: Vec<ProofChain>,
    /// Canonically ordered missing relationships.
    pub gaps: Vec<Gap>,
    /// Canonically ordered unresolved conflicts.
    pub conflicts: Vec<Conflict>,
}

/// Typed protocol failure code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// Caller and worker implement incompatible protocol versions.
    UnsupportedProtocol,
    /// Input is syntactically valid JSON but violates the protocol contract.
    InvalidRequest,
    /// A referenced record is absent from the supplied slice.
    MissingRecord,
    /// Input exceeded a configured resource boundary.
    ResourceLimit,
    /// The worker encountered an internal failure.
    Internal,
}

/// Protocol failure returned without a proof verdict.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProtocolError {
    /// Stable error category.
    pub code: ErrorCode,
    /// Deterministic diagnostic safe for callers.
    pub message: String,
}

/// Response returned by the proof kernel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum KernelResponse {
    /// Successful proof evaluation.
    Proof {
        /// Protocol version implemented by the worker.
        protocol_version: u16,
        /// Correlation identifier copied from the request.
        request_id: String,
        /// Evidence snapshot evaluated by the worker.
        snapshot_id: String,
        /// Deterministic proof result.
        proof: ProofResult,
    },
    /// Typed failure that carries no verdict.
    Error {
        /// Protocol version implemented by the worker.
        protocol_version: u16,
        /// Correlation identifier copied from the request when available.
        request_id: String,
        /// Typed protocol failure.
        error: ProtocolError,
    },
}

#[cfg(test)]
mod tests {
    use super::{KernelRequest, KernelResponse, NodeKind, PROTOCOL_VERSION};

    #[test]
    fn protocol_version_starts_at_one() {
        assert_eq!(PROTOCOL_VERSION, 1);
    }

    #[test]
    fn strict_request_rejects_unknown_fields() {
        let request = r#"{
            "protocol_version": 1,
            "request_id": "request-1",
            "snapshot_id": "snapshot-1",
            "goal": {"target_id": "code-1", "anchor_kinds": ["decision"]},
            "nodes": [],
            "edges": [],
            "conflicts": [],
            "unexpected": true
        }"#;

        assert!(serde_json::from_str::<KernelRequest>(request).is_err());
    }

    #[test]
    fn strict_response_rejects_unknown_fields() {
        let response = r#"{
            "status": "error",
            "protocol_version": 1,
            "request_id": "request-1",
            "error": {"code": "invalid_request", "message": "invalid"},
            "unexpected": true
        }"#;

        assert!(serde_json::from_str::<KernelResponse>(response).is_err());
    }

    #[test]
    fn enums_reject_unknown_values() {
        assert!(serde_json::from_str::<NodeKind>(r#""future_record""#).is_err());
    }
}
