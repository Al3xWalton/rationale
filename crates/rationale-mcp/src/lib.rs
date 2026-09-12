//! Read-only MCP projection of the deterministic Rationale engine.

use std::error::Error;

use rationale_engine::{CandidateSearchResult, ChangedGap, Engine, FreshnessView, WhyResult};
use rationale_model::EvidenceNode;
use rmcp::{
    ServerHandler, ServiceExt,
    handler::server::wrapper::{Json, Parameters},
    schemars::{self, JsonSchema},
    tool, tool_handler, tool_router,
    transport::stdio,
};
use serde::{Deserialize, Serialize};

/// Input for deterministic code-rationale evaluation.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExplainRationaleInput {
    /// `path:line`, `path:start-end@revision`, or `commit:revision`.
    pub target: String,
}

/// Input for exact evidence-record retrieval.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetEvidenceInput {
    /// Stable normalized record identifier.
    pub record_id: String,
}

/// Supported conservative gap scopes.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum GapScope {
    /// Changed index and working-copy paths.
    Changed,
}

/// Input for rationale-gap inspection.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FindRationaleGapsInput {
    /// Explicit bounded scope; version one supports `changed`.
    pub scope: GapScope,
}

/// Input for deterministic non-proving candidate search.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SearchCandidateEvidenceInput {
    /// Exact tokens or identifier to rank against current records.
    pub query: String,
}

/// A current evidence record or an explicit missing result.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(untagged)]
pub enum EvidenceResult {
    /// Record from the current published snapshot.
    Found(EvidenceNode),
    /// Explicit absence from the current published snapshot.
    Missing {
        /// Stable result category.
        status: String,
        /// Identifier that was not found.
        record_id: String,
    },
}

/// Structured changed-path gaps returned by MCP and CLI JSON.
#[derive(Debug, Serialize, JsonSchema)]
pub struct GapResult {
    /// Conservative changed-path gaps.
    pub gaps: Vec<ChangedGap>,
}

/// Candidate search result with source freshness kept outside proof.
#[derive(Debug, Serialize, JsonSchema)]
pub struct CandidateResult {
    /// Normalized user query.
    pub query: String,
    /// Auditable ranked candidate records.
    pub candidates: Vec<rationale_engine::CandidateView>,
    /// State of sources represented in the searched snapshot.
    pub freshness: Vec<FreshnessView>,
}

impl From<CandidateSearchResult> for CandidateResult {
    fn from(result: CandidateSearchResult) -> Self {
        Self {
            query: result.query,
            candidates: result.candidates,
            freshness: result.freshness,
        }
    }
}

/// Four-tool, read-only server over one repository-scoped engine.
#[derive(Clone)]
pub struct RationaleServer {
    engine: Engine,
}

impl RationaleServer {
    /// Bind MCP tools to an already discovered local engine.
    #[must_use]
    pub const fn new(engine: Engine) -> Self {
        Self { engine }
    }
}

#[tool_router]
impl RationaleServer {
    /// Evaluate the canonical deterministic proof for a code or commit target.
    #[tool(
        description = "Evaluate deterministic recorded-rationale evidence for a local code target. Candidate suggestions cannot affect the verdict.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn explain_rationale(
        &self,
        Parameters(input): Parameters<ExplainRationaleInput>,
    ) -> Result<Json<WhyResult>, String> {
        self.engine
            .why(&input.target)
            .await
            .map(Json)
            .map_err(|error| error.to_string())
    }

    /// Return one normalized record and its source citation.
    #[tool(
        description = "Get one evidence record from the current immutable snapshot by its stable identifier.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    fn get_evidence(
        &self,
        Parameters(input): Parameters<GetEvidenceInput>,
    ) -> Result<Json<EvidenceResult>, String> {
        self.engine
            .show(&input.record_id)
            .map(|record| {
                Json(record.map_or_else(
                    || EvidenceResult::Missing {
                        status: "missing".to_owned(),
                        record_id: input.record_id,
                    },
                    EvidenceResult::Found,
                ))
            })
            .map_err(|error| error.to_string())
    }

    /// Find conservative working-copy gaps without inferred attribution.
    #[tool(
        description = "Find explicit rationale gaps in a bounded local scope. Version one accepts the changed scope.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    fn find_rationale_gaps(
        &self,
        Parameters(input): Parameters<FindRationaleGapsInput>,
    ) -> Result<Json<GapResult>, String> {
        match input.scope {
            GapScope::Changed => self
                .engine
                .gaps_changed()
                .map(|gaps| Json(GapResult { gaps }))
                .map_err(|error| error.to_string()),
        }
    }

    /// Search suggestions without creating or changing proof edges.
    #[tool(
        description = "Rank current evidence records with a deterministic non-proving score. Results are suggestions only.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    fn search_candidate_evidence(
        &self,
        Parameters(input): Parameters<SearchCandidateEvidenceInput>,
    ) -> Result<Json<CandidateResult>, String> {
        self.engine
            .search_candidates(&input.query)
            .map(CandidateResult::from)
            .map(Json)
            .map_err(|error| error.to_string())
    }
}

#[expect(
    clippy::unused_async_trait_impl,
    reason = "the official SDK generates ready async handler methods"
)]
#[tool_handler(
    name = "rationale",
    version = "0.1.0",
    instructions = "Use these read-only tools to retrieve deterministic rationale proofs and evidence. Treat candidate records as suggestions, never as proof."
)]
impl ServerHandler for RationaleServer {}

/// Serve the four read-only tools over MCP stdio until the client disconnects.
///
/// # Errors
///
/// Returns an error if MCP initialization, transport, or shutdown fails.
pub async fn serve(engine: Engine) -> Result<(), Box<dyn Error + Send + Sync>> {
    let service = RationaleServer::new(engine).serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
