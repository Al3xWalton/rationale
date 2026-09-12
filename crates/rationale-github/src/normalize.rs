use std::collections::{BTreeMap, BTreeSet};

use rationale_model::{
    EdgeKind, EvidenceEdge, EvidenceNode, NodeKind, Origin, RecordStatus, SourceKind,
};
use serde::Deserialize;

use crate::{GitHubCursor, GitHubEvidence, GitHubRepository, RateLimit};

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct ApiItem {
    number: u64,
    #[serde(default)]
    body: Option<String>,
    html_url: String,
    #[serde(default)]
    pull_request: Option<PullMarker>,
}

#[derive(Clone, Debug, Deserialize)]
struct PullMarker {
    #[serde(rename = "url")]
    _url: String,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct ApiCommit {
    sha: String,
    html_url: String,
}

impl ApiItem {
    pub(crate) const fn number(&self) -> u64 {
        self.number
    }

    pub(crate) const fn is_pull(&self) -> bool {
        self.pull_request.is_some()
    }
}

pub(crate) fn build_evidence(
    repository: &GitHubRepository,
    items: &[ApiItem],
    commits: &BTreeMap<u64, Vec<ApiCommit>>,
    cursor: GitHubCursor,
    rate_limit: RateLimit,
) -> GitHubEvidence {
    let mut records = BTreeMap::new();
    let mut edges = BTreeMap::new();
    let issue_numbers: BTreeSet<_> = items
        .iter()
        .filter(|item| item.pull_request.is_none())
        .map(|item| item.number)
        .collect();

    for item in items {
        let is_pull = item.pull_request.is_some();
        let id = if is_pull {
            pull_id(item.number)
        } else {
            issue_id(item.number)
        };
        let origin = github_origin(&item.html_url);
        records.insert(
            id.clone(),
            EvidenceNode {
                id: id.clone(),
                kind: if is_pull {
                    NodeKind::Review
                } else {
                    NodeKind::WorkItem
                },
                status: RecordStatus::Current,
                subject_id: None,
                outcome_id: None,
                origin: origin.clone(),
            },
        );
        if !is_pull {
            continue;
        }
        for issue_number in
            explicit_closing_references(item.body.as_deref().unwrap_or_default(), repository)
        {
            if issue_numbers.contains(&issue_number) {
                let edge = edge(
                    EdgeKind::ResolvesIssue,
                    &id,
                    &issue_id(issue_number),
                    &origin,
                );
                edges.insert(edge.id.clone(), edge);
            }
        }
        for commit in commits.get(&item.number).into_iter().flatten() {
            let commit_origin = github_origin(&commit.html_url);
            records.entry(commit.sha.clone()).or_insert(EvidenceNode {
                id: commit.sha.clone(),
                kind: NodeKind::Change,
                status: RecordStatus::Current,
                subject_id: None,
                outcome_id: None,
                origin: commit_origin,
            });
            let edge = edge(EdgeKind::IncludedInPr, &commit.sha, &id, &origin);
            edges.insert(edge.id.clone(), edge);
        }
    }

    GitHubEvidence {
        records: records.into_values().collect(),
        edges: edges.into_values().collect(),
        cursor,
        rate_limit,
    }
}

fn explicit_closing_references(body: &str, repository: &GitHubRepository) -> BTreeSet<u64> {
    let tokens: Vec<_> = body.split_whitespace().collect();
    let mut issues = BTreeSet::new();
    for pair in tokens.windows(2) {
        let keyword = pair[0]
            .trim_matches(|character: char| !character.is_ascii_alphabetic())
            .to_ascii_lowercase();
        if !matches!(
            keyword.as_str(),
            "close"
                | "closes"
                | "closed"
                | "fix"
                | "fixes"
                | "fixed"
                | "resolve"
                | "resolves"
                | "resolved"
        ) {
            continue;
        }
        let reference = pair[1].trim_matches(|character: char| {
            matches!(character, ',' | '.' | ';' | ':' | '(' | ')' | '[' | ']')
        });
        if let Some(number) = parse_reference(reference, repository) {
            issues.insert(number);
        }
    }
    issues
}

fn parse_reference(reference: &str, repository: &GitHubRepository) -> Option<u64> {
    if let Some(number) = reference.strip_prefix('#') {
        return number.parse().ok();
    }
    let slug_prefix = format!("{}#", repository.slug());
    if let Some(number) = reference
        .to_ascii_lowercase()
        .strip_prefix(&slug_prefix)
        .map(str::to_owned)
    {
        return number.parse().ok();
    }
    let url_prefix = format!("https://github.com/{}/issues/", repository.slug());
    reference
        .to_ascii_lowercase()
        .strip_prefix(&url_prefix)
        .and_then(|number| number.parse().ok())
}

fn github_origin(locator: &str) -> Origin {
    Origin {
        source_kind: SourceKind::GitHub,
        locator: locator.to_owned(),
        revision: None,
        observed_at: None,
    }
}

fn issue_id(number: u64) -> String {
    format!("#{number}")
}

fn pull_id(number: u64) -> String {
    format!("PR-{number}")
}

fn edge(kind: EdgeKind, source: &str, target: &str, origin: &Origin) -> EvidenceEdge {
    let material = format!("{kind:?}\0{source}\0{target}\0{}", origin.locator);
    EvidenceEdge {
        id: format!("edge:{}", blake3::hash(material.as_bytes()).to_hex()),
        kind,
        source_id: source.to_owned(),
        target_id: target.to_owned(),
        status: RecordStatus::Current,
        origin: origin.clone(),
    }
}

#[cfg(test)]
pub(crate) fn closing_references(body: &str, repository: &GitHubRepository) -> BTreeSet<u64> {
    explicit_closing_references(body, repository)
}
