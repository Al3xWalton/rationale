//! Shared domain and protocol types for Rationale.

mod protocol;

pub use protocol::{
    Conflict, EdgeKind, ErrorCode, EvidenceEdge, EvidenceNode, Gap, GapCode, KernelRequest,
    KernelResponse, NodeKind, Origin, PROTOCOL_VERSION, ProofChain, ProofGoal, ProofResult,
    ProtocolError, RecordStatus, SourceKind, Verdict,
};
