use std::{collections::BTreeSet, str::FromStr};

use jiff::Timestamp;
use rationale_model::{EvidenceNode, RecordStatus};

use crate::{CandidateScoreComponents, CandidateView};

const SCORE_VERSION: u16 = 1;
const MAX_CANDIDATES: usize = 10;
const MAX_TOKEN_OVERLAP: usize = 8;
const MAX_PATH_OVERLAP: usize = 4;
const DAY_SECONDS: i64 = 86_400;

#[derive(Clone, Copy)]
pub(super) struct CandidateQuery<'a> {
    pub(super) exact: &'a str,
    pub(super) text: &'a str,
    pub(super) path: Option<&'a str>,
}

pub(super) fn rank_candidates(
    query: CandidateQuery<'_>,
    records: &[EvidenceNode],
    excluded: &BTreeSet<String>,
) -> Vec<CandidateView> {
    let query_tokens = tokens(query.text);
    let query_segments = query.path.map_or_else(BTreeSet::new, path_segments);
    let newest = records
        .iter()
        .filter_map(observed_second)
        .max()
        .unwrap_or(0);
    let mut ranked = Vec::new();
    for record in records {
        if excluded.contains(&record.id) || record.status == RecordStatus::Historical {
            continue;
        }
        let record_tokens = tokens(&record_text(record));
        let token_overlap = overlap(&query_tokens, &record_tokens, MAX_TOKEN_OVERLAP);
        let record_segments = path_segments(&record.origin.locator);
        let path_segment_overlap = overlap(&query_segments, &record_segments, MAX_PATH_OVERLAP);
        let exact_identifier = query.exact.trim().eq_ignore_ascii_case(&record.id);
        if !exact_identifier && token_overlap == 0 && path_segment_overlap == 0 {
            continue;
        }
        let components = CandidateScoreComponents {
            exact_identifier,
            token_overlap,
            path_segment_overlap,
            source_recency: recency_bucket(newest, observed_second(record)),
        };
        let score = components.score_v1();
        ranked.push(CandidateView {
            record_id: record.id.clone(),
            score_version: SCORE_VERSION,
            score,
            reason: reason(&components),
            components,
        });
    }
    ranked.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| left.record_id.cmp(&right.record_id))
    });
    ranked.truncate(MAX_CANDIDATES);
    ranked
}

fn record_text(record: &EvidenceNode) -> String {
    let mut text = record.id.clone();
    for value in [
        record.subject_id.as_deref(),
        record.outcome_id.as_deref(),
        Some(record.origin.locator.as_str()),
    ]
    .into_iter()
    .flatten()
    {
        text.push(' ');
        text.push_str(value);
    }
    text
}

fn tokens(text: &str) -> BTreeSet<String> {
    text.split(|character: char| !character.is_ascii_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(str::to_ascii_lowercase)
        .collect()
}

fn path_segments(path: &str) -> BTreeSet<String> {
    path.split_once('#')
        .map_or(path, |(path, _fragment)| path)
        .split('/')
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .map(str::to_ascii_lowercase)
        .collect()
}

fn overlap(left: &BTreeSet<String>, right: &BTreeSet<String>, limit: usize) -> u16 {
    u16::try_from(left.intersection(right).count().min(limit))
        .expect("bounded overlap always fits u16")
}

fn observed_second(record: &EvidenceNode) -> Option<i64> {
    record
        .origin
        .observed_at
        .as_deref()
        .and_then(|value| Timestamp::from_str(value).ok())
        .map(Timestamp::as_second)
}

fn recency_bucket(newest: i64, observed: Option<i64>) -> u16 {
    let Some(observed) = observed else {
        return 0;
    };
    if newest <= 0 {
        return 0;
    }
    let age = newest.saturating_sub(observed).max(0);
    if age <= 7 * DAY_SECONDS {
        4
    } else if age <= 30 * DAY_SECONDS {
        3
    } else if age <= 180 * DAY_SECONDS {
        2
    } else {
        u16::from(age <= 365 * DAY_SECONDS)
    }
}

fn reason(components: &CandidateScoreComponents) -> String {
    let mut reasons = Vec::new();
    if components.exact_identifier {
        reasons.push("exact identifier".to_owned());
    }
    if components.token_overlap > 0 {
        reasons.push(format!("{} exact tokens", components.token_overlap));
    }
    if components.path_segment_overlap > 0 {
        reasons.push(format!("{} path segments", components.path_segment_overlap));
    }
    if components.source_recency > 0 {
        reasons.push(format!("recency bucket {}", components.source_recency));
    }
    reasons.join(", ")
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use rationale_model::{EvidenceNode, NodeKind, Origin, RecordStatus, SourceKind};

    use super::{CandidateQuery, rank_candidates};

    fn node(id: &str, subject: &str, locator: &str, observed_at: Option<&str>) -> EvidenceNode {
        EvidenceNode {
            id: id.to_owned(),
            kind: NodeKind::WorkItem,
            status: RecordStatus::Current,
            subject_id: Some(subject.to_owned()),
            outcome_id: None,
            origin: Origin {
                source_kind: SourceKind::Document,
                locator: locator.to_owned(),
                revision: None,
                observed_at: observed_at.map(str::to_owned),
            },
        }
    }

    fn rank(records: &[EvidenceNode]) -> Vec<crate::CandidateView> {
        rank_candidates(
            CandidateQuery {
                exact: "checkout retry policy",
                text: "checkout retry policy",
                path: Some("docs/checkout.md"),
            },
            records,
            &BTreeSet::new(),
        )
    }

    #[test]
    fn input_order_is_ignored_and_equal_scores_use_record_id() {
        let left = node(
            "STORY-B",
            "checkout retry policy",
            "stories/b.md#rationale",
            None,
        );
        let right = node(
            "STORY-A",
            "checkout retry policy",
            "stories/a.md#rationale",
            None,
        );
        let forward = rank(&[left.clone(), right.clone()]);
        let reverse = rank(&[right, left]);
        assert_eq!(forward, reverse);
        assert_eq!(forward[0].record_id, "STORY-A");
        assert_eq!(forward[1].record_id, "STORY-B");
    }

    #[test]
    fn every_score_is_reconstructible_from_returned_components() {
        let recent = node(
            "STORY-9",
            "checkout retry policy",
            "docs/checkout.md#rationale",
            Some("2026-09-12T00:00:00Z"),
        );
        let ranked = rank(&[recent]);
        assert_eq!(ranked.len(), 1);
        assert_eq!(ranked[0].score_version, 1);
        assert_eq!(ranked[0].score, ranked[0].components.score_v1());
        assert_eq!(ranked[0].components.token_overlap, 3);
        assert_eq!(ranked[0].components.path_segment_overlap, 2);
    }

    #[test]
    fn excluded_and_unrelated_records_never_become_candidates() {
        let matching = node("STORY-9", "checkout retry policy", "docs/story.md", None);
        let unrelated = node("ADR-7", "database durability", "decisions/db.md", None);
        let excluded = BTreeSet::from([matching.id.clone()]);
        let ranked = rank_candidates(
            CandidateQuery {
                exact: "src/checkout.rs:4",
                text: "checkout retry policy",
                path: Some("src/checkout.rs"),
            },
            &[matching, unrelated],
            &excluded,
        );
        assert!(ranked.is_empty());
    }
}
