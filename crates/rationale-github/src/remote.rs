use crate::GitHubError;

/// Canonical GitHub owner and repository identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitHubRepository {
    owner: String,
    name: String,
}

impl GitHubRepository {
    /// Normalize an HTTPS, SSH, or scp-style `github.com` Git remote.
    ///
    /// # Errors
    ///
    /// Returns `GitHubError::InvalidRemote` when the host, owner, or repository
    /// cannot be identified without ambiguity.
    pub fn from_remote(remote: &str) -> Result<Self, GitHubError> {
        let remote = remote.trim().trim_end_matches('/');
        let path = if let Some(path) = remote.strip_prefix("git@github.com:") {
            path
        } else if let Some(path) = remote.strip_prefix("ssh://git@github.com/") {
            path
        } else if let Some(path) = remote.strip_prefix("https://github.com/") {
            path
        } else if let Some(path) = remote.strip_prefix("http://github.com/") {
            path
        } else if let Some(path) = remote.strip_prefix("git://github.com/") {
            path
        } else {
            return Err(invalid("only explicit github.com remotes are supported"));
        };
        if path.contains(['?', '#', '@']) {
            return Err(invalid("remote contains unsupported URL components"));
        }
        let path = path.strip_suffix(".git").unwrap_or(path);
        let mut parts = path.split('/');
        let owner = parts.next().unwrap_or_default();
        let name = parts.next().unwrap_or_default();
        if parts.next().is_some() || !valid_component(owner) || !valid_component(name) {
            return Err(invalid(
                "remote must identify exactly one owner and repository",
            ));
        }
        Ok(Self {
            owner: owner.to_ascii_lowercase(),
            name: name.to_ascii_lowercase(),
        })
    }

    /// Canonical lower-case owner.
    #[must_use]
    pub fn owner(&self) -> &str {
        &self.owner
    }

    /// Canonical lower-case repository name without `.git`.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Canonical `owner/name` slug.
    #[must_use]
    pub fn slug(&self) -> String {
        format!("{}/{}", self.owner, self.name)
    }
}

fn valid_component(value: &str) -> bool {
    !value.is_empty()
        && value != "."
        && value != ".."
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
        })
}

fn invalid(detail: &str) -> GitHubError {
    GitHubError::InvalidRemote {
        detail: detail.to_owned(),
    }
}
