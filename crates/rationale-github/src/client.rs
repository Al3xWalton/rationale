use std::{collections::BTreeMap, env, fmt, time::Duration};

use reqwest::{
    Client, Response, StatusCode, Url,
    header::{
        ACCEPT, AUTHORIZATION, ETAG, HeaderMap, HeaderName, HeaderValue, IF_NONE_MATCH, LINK,
        USER_AGENT,
    },
};
use serde::de::DeserializeOwned;

use crate::{
    GitHubCursor, GitHubError, GitHubRepository, GitHubSyncOutcome, MAX_PAGES, MAX_RECORDS,
    MAX_REQUESTS, MAX_RESPONSE_BYTES, RateLimit, merge_rate_limit,
    normalize::{ApiCommit, ApiItem, build_evidence},
};

const DEFAULT_API_BASE: &str = "https://api.github.com/";
const API_VERSION: &str = "2026-03-10";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

/// Reusable read-only client for the versioned GitHub REST API.
#[derive(Clone)]
pub struct GitHubClient {
    client: Client,
    api_base: Url,
}

impl fmt::Debug for GitHubClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GitHubClient")
            .field("client", &"[configured]")
            .field("api_base", &self.api_base.as_str())
            .field("authorization", &"[redacted]")
            .finish()
    }
}

impl GitHubClient {
    /// Create a client using `GH_TOKEN` or `GITHUB_TOKEN` without persisting it.
    ///
    /// # Errors
    ///
    /// Returns `GitHubError::ClientConfiguration` for invalid HTTP settings.
    pub fn from_environment() -> Result<Self, GitHubError> {
        let token = env::var("GH_TOKEN")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .or_else(|| {
                env::var("GITHUB_TOKEN")
                    .ok()
                    .filter(|value| !value.trim().is_empty())
            });
        Self::with_api_base(DEFAULT_API_BASE, token.as_deref())
    }

    /// Create a client for an explicit API base, primarily for hermetic fixture
    /// servers.
    ///
    /// The optional token is installed as a sensitive header and is never
    /// included in cursors or error values.
    ///
    /// # Errors
    ///
    /// Returns `GitHubError::ClientConfiguration` for an invalid URL, token, or
    /// HTTP client setting.
    pub fn with_api_base(api_base: &str, token: Option<&str>) -> Result<Self, GitHubError> {
        let mut headers = HeaderMap::new();
        headers.insert(
            ACCEPT,
            HeaderValue::from_static("application/vnd.github+json"),
        );
        headers.insert(
            HeaderName::from_static("x-github-api-version"),
            HeaderValue::from_static(API_VERSION),
        );
        headers.insert(USER_AGENT, HeaderValue::from_static("rationale/0.1"));
        if let Some(token) = token {
            let mut authorization = HeaderValue::from_str(&format!("Bearer {}", token.trim()))
                .map_err(|_| GitHubError::ClientConfiguration)?;
            authorization.set_sensitive(true);
            headers.insert(AUTHORIZATION, authorization);
        }
        let mut api_base = Url::parse(api_base).map_err(|_| GitHubError::ClientConfiguration)?;
        if !matches!(api_base.scheme(), "http" | "https")
            || api_base.host_str().is_none()
            || !api_base.username().is_empty()
            || api_base.password().is_some()
            || api_base.query().is_some()
            || api_base.fragment().is_some()
        {
            return Err(GitHubError::ClientConfiguration);
        }
        if !api_base.path().ends_with('/') {
            let path = format!("{}/", api_base.path());
            api_base.set_path(&path);
        }
        let client = Client::builder()
            .default_headers(headers)
            .timeout(REQUEST_TIMEOUT)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| GitHubError::ClientConfiguration)?;
        Ok(Self { client, api_base })
    }

    /// Synchronize issues, pull requests, and pull-request commit membership.
    ///
    /// The collection `ETag` makes an unchanged run conditional. Pagination is
    /// bounded and every next link must remain on the configured API origin and
    /// repository path.
    ///
    /// # Errors
    ///
    /// Returns a sanitized `GitHubError` for transport, status, schema, cursor,
    /// pagination, or resource-bound failures.
    pub async fn synchronize(
        &self,
        repository: &GitHubRepository,
        cursor: Option<&str>,
    ) -> Result<GitHubSyncOutcome, GitHubError> {
        let cursor = GitHubCursor::decode(cursor)?;
        let issues_path = format!(
            "repos/{}/issues?state=all&sort=updated&direction=desc&per_page=100",
            repository.slug()
        );
        let mut requests = 0;
        let issues = self
            .collection::<ApiItem>(
                repository,
                &issues_path,
                cursor.issues_etag.as_deref(),
                &mut requests,
            )
            .await?;
        let Collection::Updated {
            items,
            etag,
            mut rate_limit,
        } = issues
        else {
            let Collection::NotModified { rate_limit } = issues else {
                unreachable!("collection has two variants");
            };
            return Ok(GitHubSyncOutcome::NotModified { cursor, rate_limit });
        };

        let mut commits = BTreeMap::new();
        for item in items.iter().filter(|item| item.is_pull()) {
            let path = format!(
                "repos/{}/pulls/{}/commits?per_page=100",
                repository.slug(),
                item.number()
            );
            let Collection::Updated {
                items: pull_commits,
                rate_limit: commit_rate,
                ..
            } = self
                .collection::<ApiCommit>(repository, &path, None, &mut requests)
                .await?
            else {
                unreachable!("commit collections are not conditional");
            };
            merge_rate_limit(&mut rate_limit, commit_rate);
            commits.insert(item.number(), pull_commits);
        }
        let cursor = GitHubCursor { issues_etag: etag };
        Ok(GitHubSyncOutcome::Updated(build_evidence(
            repository, &items, &commits, cursor, rate_limit,
        )))
    }

    async fn collection<T: DeserializeOwned>(
        &self,
        repository: &GitHubRepository,
        initial_path: &str,
        etag: Option<&str>,
        requests: &mut usize,
    ) -> Result<Collection<T>, GitHubError> {
        let mut url = self
            .api_base
            .join(initial_path)
            .map_err(|_| GitHubError::InvalidPagination)?;
        let mut all_items = Vec::new();
        let mut first_etag = None;
        let mut rate_limit = RateLimit::default();
        for page in 0..MAX_PAGES {
            *requests += 1;
            if *requests > MAX_REQUESTS {
                return Err(GitHubError::ResourceLimit {
                    resource: "request",
                    limit: MAX_REQUESTS,
                });
            }
            let page_result = self
                .page::<T>(&url, (page == 0).then_some(etag).flatten())
                .await?;
            let Page::Updated {
                items,
                next,
                etag,
                rate_limit: page_rate,
            } = page_result
            else {
                let Page::NotModified { rate_limit } = page_result else {
                    unreachable!("page has two variants");
                };
                return Ok(Collection::NotModified { rate_limit });
            };
            if page == 0 {
                first_etag = etag;
            }
            merge_rate_limit(&mut rate_limit, page_rate);
            all_items.extend(items);
            if all_items.len() > MAX_RECORDS {
                return Err(GitHubError::ResourceLimit {
                    resource: "record",
                    limit: MAX_RECORDS,
                });
            }
            let Some(next) = next else {
                return Ok(Collection::Updated {
                    items: all_items,
                    etag: first_etag,
                    rate_limit,
                });
            };
            url = self.validate_next(repository, &next)?;
        }
        Err(GitHubError::ResourceLimit {
            resource: "page",
            limit: MAX_PAGES,
        })
    }

    async fn page<T: DeserializeOwned>(
        &self,
        url: &Url,
        etag: Option<&str>,
    ) -> Result<Page<T>, GitHubError> {
        let endpoint = url.path().to_owned();
        let mut request = self.client.get(url.clone());
        if let Some(etag) = etag {
            request = request.header(IF_NONE_MATCH, etag);
        }
        let response = request
            .send()
            .await
            .map_err(|error| GitHubError::Transport {
                endpoint: endpoint.clone(),
                category: transport_category(&error),
            })?;
        let rate_limit = response_rate_limit(&response);
        if response.status() == StatusCode::NOT_MODIFIED {
            return Ok(Page::NotModified { rate_limit });
        }
        if !response.status().is_success() {
            return Err(GitHubError::HttpStatus {
                endpoint,
                status: response.status().as_u16(),
            });
        }
        let etag = header_string(&response, ETAG);
        let next = header_string(&response, LINK).and_then(|link| next_link(&link));
        let body = bounded_body(response, &endpoint).await?;
        let items =
            serde_json::from_slice(&body).map_err(|_| GitHubError::InvalidResponse { endpoint })?;
        Ok(Page::Updated {
            items,
            next,
            etag,
            rate_limit,
        })
    }

    fn validate_next(&self, repository: &GitHubRepository, next: &str) -> Result<Url, GitHubError> {
        let url = Url::parse(next).map_err(|_| GitHubError::InvalidPagination)?;
        let scope = self
            .api_base
            .join(&format!("repos/{}/", repository.slug()))
            .map_err(|_| GitHubError::InvalidPagination)?;
        let same_origin = url.scheme() == scope.scheme()
            && url.host_str() == scope.host_str()
            && url.port_or_known_default() == scope.port_or_known_default()
            && url.username().is_empty()
            && url.password().is_none();
        if !same_origin || !url.path().starts_with(scope.path()) {
            return Err(GitHubError::InvalidPagination);
        }
        Ok(url)
    }
}

enum Collection<T> {
    Updated {
        items: Vec<T>,
        etag: Option<String>,
        rate_limit: RateLimit,
    },
    NotModified {
        rate_limit: RateLimit,
    },
}

enum Page<T> {
    Updated {
        items: Vec<T>,
        next: Option<String>,
        etag: Option<String>,
        rate_limit: RateLimit,
    },
    NotModified {
        rate_limit: RateLimit,
    },
}

async fn bounded_body(mut response: Response, endpoint: &str) -> Result<Vec<u8>, GitHubError> {
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| GitHubError::Transport {
            endpoint: endpoint.to_owned(),
            category: transport_category(&error),
        })?
    {
        if body.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
            return Err(GitHubError::ResourceLimit {
                resource: "response byte",
                limit: MAX_RESPONSE_BYTES,
            });
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

fn header_string(response: &Response, name: reqwest::header::HeaderName) -> Option<String> {
    response
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

fn response_rate_limit(response: &Response) -> RateLimit {
    RateLimit {
        limit: numeric_header(response, "x-ratelimit-limit"),
        remaining: numeric_header(response, "x-ratelimit-remaining"),
        reset_at: numeric_header(response, "x-ratelimit-reset")
            .and_then(|value| i64::try_from(value).ok()),
    }
}

fn numeric_header(response: &Response, name: &'static str) -> Option<u64> {
    response
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse().ok())
}

fn next_link(value: &str) -> Option<String> {
    value.split(',').find_map(|part| {
        let mut pieces = part.split(';');
        let url = pieces.next()?.trim();
        let is_next = pieces.any(|piece| piece.trim() == "rel=\"next\"");
        is_next
            .then(|| url.strip_prefix('<')?.strip_suffix('>').map(str::to_owned))
            .flatten()
    })
}

fn transport_category(error: &reqwest::Error) -> &'static str {
    if error.is_timeout() {
        "timeout"
    } else if error.is_connect() {
        "connection"
    } else if error.is_body() {
        "body"
    } else {
        "transport"
    }
}
