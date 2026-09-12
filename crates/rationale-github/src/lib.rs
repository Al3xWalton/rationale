//! Read-only synchronization of explicit GitHub issue and pull-request evidence.

use rationale_model::{EvidenceEdge, EvidenceNode};
use serde::{Deserialize, Serialize};
use thiserror::Error;

mod client;
mod normalize;
mod remote;

pub use client::GitHubClient;
pub use remote::GitHubRepository;

const MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;
const MAX_PAGES: usize = 100;
const MAX_RECORDS: usize = 10_000;
const MAX_REQUESTS: usize = 1_000;

/// Failure to identify, retrieve, or normalize GitHub evidence.
#[derive(Debug, Error)]
pub enum GitHubError {
    /// A Git remote is unsupported or ambiguous.
    #[error("invalid GitHub remote: {detail}")]
    InvalidRemote {
        /// Stable sanitized validation detail.
        detail: String,
    },
    /// The HTTP client could not be constructed.
    #[error("GitHub HTTP client configuration failed")]
    ClientConfiguration,
    /// A request failed before GitHub returned a response.
    #[error("GitHub request failed for {endpoint}: {category}")]
    Transport {
        /// API path without credentials or query secrets.
        endpoint: String,
        /// Coarse sanitized failure category.
        category: &'static str,
    },
    /// GitHub returned a non-success status.
    #[error("GitHub returned HTTP {status} for {endpoint}")]
    HttpStatus {
        /// API path without credentials or query secrets.
        endpoint: String,
        /// Numeric HTTP status.
        status: u16,
    },
    /// A response exceeded a local bound.
    #[error("GitHub synchronization exceeded the {resource} limit of {limit}")]
    ResourceLimit {
        /// Bounded resource category.
        resource: &'static str,
        /// Configured maximum.
        limit: usize,
    },
    /// A response did not match the supported GitHub schema.
    #[error("GitHub returned invalid JSON for {endpoint}")]
    InvalidResponse {
        /// API path without credentials or query secrets.
        endpoint: String,
    },
    /// GitHub supplied an unsafe pagination URL.
    #[error("GitHub returned an invalid pagination link")]
    InvalidPagination,
    /// A stored conditional-request cursor was malformed.
    #[error("stored GitHub synchronization cursor is invalid")]
    InvalidCursor,
}

/// Primary rate-limit metadata copied from GitHub response headers.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RateLimit {
    /// Hourly request limit, when supplied.
    pub limit: Option<u64>,
    /// Requests remaining, taking the minimum across the synchronization.
    pub remaining: Option<u64>,
    /// UTC reset time in Unix seconds, when supplied.
    pub reset_at: Option<i64>,
}

/// Opaque resumable state safe to persist without credentials.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct GitHubCursor {
    /// `ETag` for the first repository-issues page.
    pub issues_etag: Option<String>,
}

impl GitHubCursor {
    /// Decode a cursor previously returned by this crate.
    ///
    /// # Errors
    ///
    /// Returns `GitHubError::InvalidCursor` for malformed or incompatible JSON.
    pub fn decode(value: Option<&str>) -> Result<Self, GitHubError> {
        value.map_or_else(
            || Ok(Self::default()),
            |value| serde_json::from_str(value).map_err(|_| GitHubError::InvalidCursor),
        )
    }

    /// Encode stable cursor JSON for transactional publication.
    ///
    /// # Errors
    ///
    /// Returns `GitHubError::InvalidCursor` if serialization unexpectedly fails.
    pub fn encode(&self) -> Result<String, GitHubError> {
        serde_json::to_string(self).map_err(|_| GitHubError::InvalidCursor)
    }
}

/// Normalized contribution from one successful GitHub synchronization.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitHubEvidence {
    /// Issue, review, and commit records in stable identifier order.
    pub records: Vec<EvidenceNode>,
    /// Commit-membership and explicit issue-resolution edges in stable order.
    pub edges: Vec<EvidenceEdge>,
    /// Cursor to commit atomically with this contribution.
    pub cursor: GitHubCursor,
    /// Most conservative rate metadata seen during the fetch.
    pub rate_limit: RateLimit,
}

/// Result of a conditional GitHub synchronization.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GitHubSyncOutcome {
    /// GitHub supplied a normalized new contribution.
    Updated(GitHubEvidence),
    /// The collection `ETag` confirms the prior contribution is still current.
    NotModified {
        /// Unchanged cursor supplied by the caller.
        cursor: GitHubCursor,
        /// Rate metadata copied from the 304 response.
        rate_limit: RateLimit,
    },
}

fn merge_rate_limit(current: &mut RateLimit, incoming: RateLimit) {
    current.limit = incoming.limit.or(current.limit);
    current.remaining = match (current.remaining, incoming.remaining) {
        (Some(left), Some(right)) => Some(left.min(right)),
        (left, right) => left.or(right),
    };
    current.reset_at = incoming.reset_at.or(current.reset_at);
}

#[cfg(test)]
mod tests;
