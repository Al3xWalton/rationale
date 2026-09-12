use crate::{ExplicitReference, ReferenceKind};

pub(crate) fn extract(message: &str) -> Vec<ExplicitReference> {
    let tokens: Vec<_> = message.split_whitespace().collect();
    let mut references = Vec::new();
    for (index, token) in tokens.iter().enumerate() {
        let cleaned = token.trim_matches(|character: char| {
            matches!(character, ',' | '.' | ';' | '(' | ')' | '[' | ']' | '`')
        });
        let previous = index
            .checked_sub(1)
            .and_then(|previous| tokens.get(previous))
            .map(|previous| {
                previous
                    .trim_matches(|character: char| !character.is_ascii_alphanumeric())
                    .to_ascii_lowercase()
            });
        if let Some(number) = cleaned.strip_prefix('#').filter(|value| digits(value)) {
            let kind = if previous.as_deref() == Some("story") {
                ReferenceKind::Story
            } else {
                ReferenceKind::Issue
            };
            references.push(ExplicitReference {
                kind,
                value: format!("#{number}"),
            });
            continue;
        }

        let uppercase = cleaned.to_ascii_uppercase();
        if let Some(number) = uppercase
            .strip_prefix("STORY-")
            .filter(|value| digits(value))
        {
            references.push(ExplicitReference {
                kind: ReferenceKind::Story,
                value: format!("STORY-{number}"),
            });
        } else if let Some(number) = uppercase.strip_prefix("ADR-").filter(|value| digits(value)) {
            references.push(ExplicitReference {
                kind: ReferenceKind::Decision,
                value: format!("ADR-{number}"),
            });
        } else if (cleaned.starts_with("https://") || cleaned.starts_with("http://"))
            && (cleaned.contains("/issues/") || cleaned.contains("/pull/"))
        {
            references.push(ExplicitReference {
                kind: ReferenceKind::ForgeUrl,
                value: cleaned.to_owned(),
            });
        }
    }
    references.sort();
    references.dedup();
    references
}

fn digits(value: &str) -> bool {
    !value.is_empty() && value.chars().all(|character| character.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use crate::{ExplicitReference, ReferenceKind};

    use super::extract;

    #[test]
    fn extracts_only_explicit_supported_references() {
        assert_eq!(
            extract(
                "Implement ADR-0007 for Story: #12; fixes #44. See \
                 https://github.com/acme/repo/pull/9 and maybe ticket 99."
            ),
            vec![
                ExplicitReference {
                    kind: ReferenceKind::Issue,
                    value: "#44".to_owned(),
                },
                ExplicitReference {
                    kind: ReferenceKind::Story,
                    value: "#12".to_owned(),
                },
                ExplicitReference {
                    kind: ReferenceKind::Decision,
                    value: "ADR-0007".to_owned(),
                },
                ExplicitReference {
                    kind: ReferenceKind::ForgeUrl,
                    value: "https://github.com/acme/repo/pull/9".to_owned(),
                },
            ]
        );
    }
}
