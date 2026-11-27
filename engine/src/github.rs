//! GitHub through `gh` (the person's own sign-in): who I am, the repo behind
//! a remote URL, and who has access. Network calls only; no state.

use serde::Serialize;
use std::process::Command;

/// owner/name of a GitHub repo.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct GitHubRepo {
    pub owner: String,
    pub name: String,
}

impl GitHubRepo {
    /// From an origin URL: git@github.com:o/r.git, https://github.com/o/r(.git), ssh://git@github.com/o/r.
    pub fn parse(url: &str) -> Option<GitHubRepo> {
        let rest = url
            .strip_prefix("git@github.com:")
            .or_else(|| url.strip_prefix("https://github.com/"))
            .or_else(|| url.strip_prefix("ssh://git@github.com/"))
            .or_else(|| url.strip_prefix("http://github.com/"))?;
        let rest = rest.trim_end_matches('/').trim_end_matches(".git");
        let (owner, name) = rest.split_once('/')?;
        if owner.is_empty() || name.is_empty() || name.contains('/') {
            return None;
        }
        Some(GitHubRepo { owner: owner.into(), name: name.into() })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GhError {
    /// `gh` is not installed.
    Missing,
    /// `gh` is installed but not signed in.
    NotSignedIn,
    Failed(String),
}

impl std::fmt::Display for GhError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GhError::Missing => write!(f, "The GitHub CLI (gh) is not installed. Install it from cli.github.com, then run gh auth login."),
            GhError::NotSignedIn => write!(f, "Not signed in to GitHub. Run gh auth login."),
            GhError::Failed(m) => write!(f, "{m}"),
        }
    }
}

fn gh(args: &[&str]) -> Result<String, GhError> {
    let out = Command::new("gh").args(args).env("GH_PROMPT_DISABLED", "1").output().map_err(|_| GhError::Missing)?;
    if out.status.success() {
        return Ok(String::from_utf8_lossy(&out.stdout).into_owned());
    }
    let err = String::from_utf8_lossy(&out.stderr).to_string();
    if err.contains("auth login") || err.contains("not logged in") {
        return Err(GhError::NotSignedIn);
    }
    Err(GhError::Failed(err.trim().to_string()))
}

/// The signed-in GitHub login. $HERDR_SESSION_LOGIN overrides it (tests, and
/// machines without gh where the person accepts a claimed login: Who can join
/// is cooperative).
pub fn login() -> Result<String, GhError> {
    if let Ok(l) = std::env::var("HERDR_SESSION_LOGIN") {
        if !l.is_empty() {
            return Ok(l);
        }
    }
    let out = gh(&["api", "user", "--jq", ".login"])?;
    let l = out.trim().to_string();
    if l.is_empty() {
        return Err(GhError::NotSignedIn);
    }
    Ok(l)
}

/// This person's access to a repo: Some("write"), Some("read"), or None
/// (no access; GitHub answers 404 for a private repo you cannot read).
pub fn permission(repo: &GitHubRepo) -> Result<Option<&'static str>, GhError> {
    let path = format!("repos/{}/{}", repo.owner, repo.name);
    match gh(&["api", &path, "--jq", "[.permissions.push, .permissions.pull] | @tsv"]) {
        Ok(out) => {
            let mut it = out.trim().split('\t');
            let push = it.next() == Some("true");
            let pull = it.next() == Some("true");
            Ok(if push { Some("write") } else if pull { Some("read") } else { None })
        }
        Err(GhError::Failed(m)) if m.contains("404") || m.contains("Not Found") => Ok(None),
        Err(e) => Err(e),
    }
}

/// The URL git should use for a repo: ssh when the person told gh to use ssh, else https
/// (which gh's credential helper signs in).
pub fn clone_url(repo: &GitHubRepo) -> String {
    let ssh = gh(&["config", "get", "git_protocol", "-h", "github.com"]).map(|p| p.trim() == "ssh").unwrap_or(false);
    if ssh {
        format!("git@github.com:{}/{}.git", repo.owner, repo.name)
    } else {
        format!("https://github.com/{}/{}.git", repo.owner, repo.name)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Collaborator {
    pub login: String,
    /// "read" or "write" (write, maintain and admin all push).
    pub permission: &'static str,
}

/// People with access to the repo, for the Who can join menu.
pub fn collaborators(repo: &GitHubRepo) -> Result<Vec<Collaborator>, GhError> {
    let path = format!("repos/{}/{}/collaborators?per_page=100", repo.owner, repo.name);
    let out = gh(&["api", "--paginate", &path, "--jq", ".[] | [.login, (.permissions.push|tostring)] | @tsv"])?;
    let mut people: Vec<Collaborator> = out
        .lines()
        .filter_map(|l| l.split_once('\t'))
        .map(|(login, push)| Collaborator { login: login.to_string(), permission: if push == "true" { "write" } else { "read" } })
        .collect();
    people.sort_by_key(|p| (p.permission != "write", p.login.to_lowercase()));
    Ok(people)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_github_remotes() {
        let want = Some(GitHubRepo { owner: "deleteme".into(), name: "joindeleterepo".into() });
        assert_eq!(GitHubRepo::parse("git@github.com:deleteme/joindeleterepo.git"), want);
        assert_eq!(GitHubRepo::parse("https://github.com/deleteme/joindeleterepo"), want);
        assert_eq!(GitHubRepo::parse("https://github.com/deleteme/joindeleterepo.git"), want);
        assert_eq!(GitHubRepo::parse("ssh://git@github.com/deleteme/joindeleterepo.git"), want);
        assert_eq!(GitHubRepo::parse("https://gitlab.com/deleteme/joindeleterepo.git"), None);
        assert_eq!(GitHubRepo::parse("file:///tmp/remote.git"), None);
    }
}
