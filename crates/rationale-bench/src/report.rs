use serde::Serialize;

use crate::{
    fixture::{Scale, ScaleConfig},
    stats::{Distribution, HostMetadata},
};

#[derive(Debug, Serialize)]
pub(crate) struct BenchmarkReport {
    pub(crate) schema_version: u16,
    pub(crate) scale: Scale,
    pub(crate) fixture: ScaleConfig,
    pub(crate) measurement_samples: usize,
    pub(crate) host: HostMetadata,
    pub(crate) metrics: Metrics,
    pub(crate) proof_scope: ProofScope,
    pub(crate) stability: Stability,
    pub(crate) budgets: Budgets,
}

#[derive(Debug, Serialize)]
pub(crate) struct Metrics {
    pub(crate) cold_local_ingestion: Distribution,
    pub(crate) incremental_local_sync: Distribution,
    pub(crate) graph_slice_extraction: Distribution,
    pub(crate) rust_protocol_codec: Distribution,
    pub(crate) ocaml_kernel_evaluation_ms: f64,
    pub(crate) worker_round_trip: Distribution,
    pub(crate) estimated_process_protocol_overhead_ms: f64,
    pub(crate) warm_engine_query: Distribution,
    pub(crate) warm_cli_query: Distribution,
    pub(crate) warm_mcp_query: Distribution,
    pub(crate) sqlite_publication: Distribution,
}

#[derive(Debug, Serialize)]
pub(crate) struct ProofScope {
    pub(crate) snapshot_nodes: usize,
    pub(crate) snapshot_edges: usize,
    pub(crate) request_nodes: usize,
    pub(crate) request_edges: usize,
}

#[derive(Debug, Serialize)]
pub(crate) struct Stability {
    pub(crate) repeated_queries: usize,
    pub(crate) rss_before_kib: u64,
    pub(crate) rss_after_kib: u64,
    pub(crate) rss_growth_kib: i64,
    pub(crate) engine_payloads_byte_identical: CheckStatus,
    pub(crate) cli_payloads_byte_identical: CheckStatus,
    pub(crate) mcp_payloads_byte_identical: CheckStatus,
    pub(crate) worker_payloads_byte_identical: CheckStatus,
}

#[derive(Debug, Serialize)]
pub(crate) struct Budgets {
    pub(crate) warm_query_p95_under_100_ms: CheckStatus,
    pub(crate) incremental_sync_p95_under_1000_ms: CheckStatus,
    pub(crate) proof_request_is_sliced: CheckStatus,
    pub(crate) repeated_rss_growth_under_8192_kib: CheckStatus,
    pub(crate) canonical_payloads_stable: CheckStatus,
    pub(crate) all_passed: CheckStatus,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CheckStatus {
    Pass,
    Fail,
}

impl CheckStatus {
    pub(crate) const fn passed(self) -> bool {
        matches!(self, Self::Pass)
    }
}

impl From<bool> for CheckStatus {
    fn from(value: bool) -> Self {
        if value { Self::Pass } else { Self::Fail }
    }
}
