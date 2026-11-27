//! Joining a session from its link, one step per call so the dialog shows
//! real progress. Every step re-checks instead of trusting an earlier one, so
//! running the whole join again (after a crash, a quit, a fix) is safe and
//! quick: done work is found and kept.
//!
//! The repository lives once per machine as a blobless bare clone in
//! `~/.herdr/session/repos/<owner>/<repo>.git`; each joined session is a
//! worktree of it in `<folder>/<repo>/<branch>` (folder: ~/herdr-sessions).

use crate::api::{ApiError, REMOTE};
use crate::git::{Checkout, Repo};
use crate::github::{self, GhError, GitHubRepo};
use crate::ledger::{decode_blob, Body, Fold, Ms, SessionId, WhoCanJoin};
use crate::session::{self, Binding};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::time::Duration;

const NET: Duration = Duration::from_secs(60);
const CLONE: Duration = Duration::from_secs(600);

fn err(kind: &'static str, message: impl Into<String>) -> ApiError {
    ApiError { kind, message: message.into() }
}

/// `https://herdr.dev/join/<owner>/<repo>/<code>`, with or without the scheme.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    pub repo: GitHubRepo,
    pub code: String,
}

impl Link {
    pub fn parse(s: &str) -> Option<Link> {
        let s = s.trim();
        let rest = s.strip_prefix("https://").or_else(|| s.strip_prefix("http://")).unwrap_or(s);
        let rest = rest.strip_prefix("herdr.dev/join/")?.trim_end_matches('/');
        let mut parts = rest.split('/');
        let (owner, name, code) = (parts.next()?, parts.next()?, parts.next()?);
        let ok = |p: &str| !p.is_empty() && p.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c));
        let code_ok = code.len() == 8 && code.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit());
        if parts.next().is_some() || !ok(owner) || !ok(name) || !code_ok {
            return None;
        }
        Some(Link { repo: GitHubRepo { owner: owner.into(), name: name.into() }, code: code.into() })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// 1. Find the session on GitHub.
    Session,
    /// 2. Signed in to GitHub.
    Github,
    /// 3. Access to the repo, and allowed by Who can join.
    Access,
    /// 4. The worktree folder can be made.
    Folder,
    /// 5. Check out the branch: the worktree, the binding, "joined", the Claude hooks.
    Branch,
}

impl Step {
    pub fn parse(s: &str) -> Option<Step> {
        Some(match s {
            "session" => Step::Session,
            "github" => Step::Github,
            "access" => Step::Access,
            "folder" => Step::Folder,
            "branch" => Step::Branch,
            _ => return None,
        })
    }
}

/// What a passed step tells the dialog. Every reply carries the session's
/// header facts, so the dialog can draw them from step 1 on.
#[derive(Debug, Serialize)]
pub struct Reply {
    pub owner: String,
    pub name: String,
    pub repo: String,
    pub branch: String,
    pub events: usize,
    /// Step 2 on.
    pub login: Option<String>,
    /// Step 3 on: "write" or "read".
    pub permission: Option<&'static str>,
    /// Step 4 on: the worktree, and the same for people (`~/…`).
    pub worktree: Option<PathBuf>,
    pub worktree_shown: Option<String>,
}

fn home_dir() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
}

pub fn default_folder() -> PathBuf {
    home_dir().join("herdr-sessions")
}

/// The machine's one clone of a repo.
fn clone_dir(repo: &GitHubRepo) -> PathBuf {
    session::home().join("repos").join(&repo.owner).join(format!("{}.git", repo.name))
}

/// The bare clone, made (empty) if absent; nothing is downloaded here.
fn ensure_clone(gh: &GitHubRepo) -> Result<Repo, ApiError> {
    let dir = clone_dir(gh);
    let repo = Repo { dir: dir.clone() };
    if !dir.join("HEAD").exists() {
        std::fs::create_dir_all(&dir).map_err(|e| err("Io", format!("Couldn't create {}: {e}", dir.display())))?;
        repo.run(&["init", "--quiet", "--bare"])?;
    }
    let url = github::clone_url(gh);
    if repo.remote_url(REMOTE).is_none() {
        repo.run(&["remote", "add", REMOTE, &url])?;
    }
    // A normal fetch refspec (bare clones have none), and blobs on demand.
    repo.run(&["config", &format!("remote.{REMOTE}.fetch"), "+refs/heads/*:refs/remotes/origin/*"])?;
    repo.run(&["config", &format!("remote.{REMOTE}.promisor"), "true"])?;
    repo.run(&["config", &format!("remote.{REMOTE}.partialclonefilter"), "blob:none"])?;
    // Git's record of pushes (how a push from a terminal is noticed); bare clones keep none by default.
    repo.run(&["config", "core.logAllRefUpdates", "true"])?;
    Ok(repo)
}

/// The session whose code this is, from the streams fetched for it.
fn find_sid(repo: &Repo, code: &str) -> Option<SessionId> {
    let refs = repo.refs(&format!("refs/herdr-seen/{code}/m/")).ok()?;
    refs.iter().find_map(|(name, _)| {
        let first = repo.read_stream(name).ok()?.into_iter().next()?;
        let sid = SessionId::from_hex(&decode_blob(&first)?.event.sid)?;
        (sid.code() == code).then_some(sid)
    })
}

/// Step 1's work, shared by every later step: fetch the session's streams and fold them.
fn session(link: &Link, now: Ms) -> Result<(Repo, Fold), ApiError> {
    let repo = ensure_clone(&link.repo)?;
    let code = &link.code;
    if let Err(e) = repo.fetch(REMOTE, &format!("+refs/herdr/s/{code}/*:refs/herdr-seen/{code}/*"), NET) {
        // GitHub hides a private repo you can't read: find out why before blaming the link.
        signed_in()?;
        match github::permission(&link.repo) {
            Ok(None) => return Err(no_access()),
            Err(GhError::NotSignedIn | GhError::Missing) => return Err(not_signed_in()),
            _ => return Err(err("Network", format!("Couldn't reach GitHub: {e}"))),
        }
    }
    let not_found = || err("SessionNotFound", "Session not found. We couldn't find a session at this link.");
    let sid = find_sid(&repo, code).ok_or_else(not_found)?;
    let fold = session::load_repo(&repo, &sid, now).map_err(|_| not_found())?;
    if !fold.is_open() {
        return Err(err("Closed", "This session is closed. Ask the owner to turn sharing back on, then try again."));
    }
    Ok((repo, fold))
}

fn not_signed_in() -> ApiError {
    err("NotSignedIn", "Not signed in to GitHub. herdr needs your GitHub login to check your access.")
}

fn no_access() -> ApiError {
    err("NoAccess", "No access to this repository. Ask the owner to add you on GitHub, then try again.")
}

fn signed_in() -> Result<String, ApiError> {
    github::login().map_err(|e| match e {
        GhError::Failed(m) => err("GitHub", m),
        _ => not_signed_in(),
    })
}

fn access(link: &Link, fold: &Fold, login: &str) -> Result<&'static str, ApiError> {
    let permission = match github::permission(&link.repo) {
        Ok(Some(p)) => p,
        Ok(None) => return Err(no_access()),
        Err(GhError::Failed(m)) => return Err(err("GitHub", m)),
        Err(_) => return Err(not_signed_in()),
    };
    let allowed = login == fold.genesis.owner
        || match &fold.settings.who_can_join {
            WhoCanJoin::Anyone => true,
            WhoCanJoin::Only { logins } => logins.contains(login),
        };
    if !allowed {
        return Err(err("NotInvited", "This session is limited to selected people. Ask the owner to add you."));
    }
    Ok(permission)
}

fn worktree_path(folder: &Path, link: &Link, branch: &str) -> PathBuf {
    folder.join(&link.repo.name).join(branch)
}

/// The worktree is ours already: a checkout of this branch bound to this session.
fn ours(path: &Path, branch: &str, sid: &SessionId) -> Option<Checkout> {
    let at = Checkout::discover(path)?;
    (at.root == path.canonicalize().ok()? && at.branch == branch && session::binding(&at)?.sid == sid.hex()).then_some(at)
}

fn folder_ok(path: &Path, sid: &SessionId, branch: &str) -> Result<(), ApiError> {
    let unusable = |_why: &str| {
        err("FolderUnusable", format!("Couldn't create {}. It may already exist, or herdr can't write there.", tilde(path)))
    };
    if ours(path, branch, sid).is_some() {
        return Ok(());
    }
    if path.exists() {
        let empty = std::fs::read_dir(path).map(|mut d| d.next().is_none()).unwrap_or(false);
        if !empty {
            return Err(unusable("It already exists."));
        }
        return Ok(());
    }
    // Make the parent and prove it's writable; git makes the worktree itself.
    let parent = path.parent().unwrap_or(path);
    std::fs::create_dir_all(parent).map_err(|_| unusable("herdr can't write there."))?;
    let probe = parent.join(format!(".herdr-probe-{}", std::process::id()));
    std::fs::write(&probe, b"").map_err(|_| unusable("herdr can't write there."))?;
    let _ = std::fs::remove_file(probe);
    Ok(())
}

fn tilde(p: &Path) -> String {
    let home = home_dir();
    match p.strip_prefix(&home) {
        Ok(rest) if !home.as_os_str().is_empty() => format!("~/{}", rest.display()),
        _ => p.display().to_string(),
    }
}

/// Step 5: the branch, the worktree, the binding, our "joined", the hooks.
fn check_out(repo: &Repo, fold: &Fold, login: &str, permission: &str, path: &Path) -> Result<(), ApiError> {
    let sid = fold.sid;
    let branch = &fold.genesis.branch;
    let gone = || err("BranchGone", format!("{branch} is no longer on GitHub. Ask the owner to push it again."));
    let at = match ours(path, branch, &sid) {
        Some(at) => at,
        None => {
            repo.run_with(
                &["fetch", "--quiet", "--no-tags", "--filter=blob:none", REMOTE, &format!("+refs/heads/{branch}:refs/remotes/origin/{branch}")],
                None,
                Some(CLONE),
            )
            .map_err(|e| if e.to_string().contains("couldn't find remote ref") { gone() } else { e.into() })?;
            let upstream = format!("origin/{branch}");
            // The branch may exist here already (another worktree of it is not allowed by git).
            let has_branch = repo.ref_oid(&format!("refs/heads/{branch}")).is_some();
            let mut args = vec!["worktree", "add", "--quiet"];
            let p = path.to_string_lossy().to_string();
            if has_branch {
                args.extend([p.as_str(), branch.as_str()]);
            } else {
                args.extend(["--track", "-b", branch.as_str(), p.as_str(), upstream.as_str()]);
            }
            repo.run_with(&args, None, Some(CLONE)).map_err(|e| err("FolderUnusable", format!("Couldn't create {}. It may already exist, or herdr can't write there. ({e})", tilde(path))))?;
            let at = Checkout::discover(path).ok_or_else(|| err("Io", "the worktree was made but git can't see it"))?;
            session::bind(&at, &Binding { sid: sid.hex(), remote: REMOTE.into(), login: login.into(), keep_in_sync: true, read_only: permission != "write" })
                .map_err(|e| err("Io", e.to_string()))?;
            at
        }
    };
    // Read-only people publish nothing (O2). Everyone else says "joined" once per device.
    if permission == "write" {
        let b = session::binding(&at).ok_or_else(|| err("Io", "the binding is missing"))?;
        let key = session::device_key(&at).map_err(|e| err("Io", e.to_string()))?;
        let mine = session::own_ref(&sid, &key.tag(&sid));
        let said = at.repo().ref_oid(&mine).is_some() || at.repo().ref_oid(&mine.replace("refs/herdr/s/", "refs/herdr-seen/")).is_some();
        if !said {
            session::append(&at, &b, Body::Joined, None, crate::util::now_ms())?;
        }
        session::sync(&at, &b, NET);
    }
    let _ = crate::claude::install_hooks();
    crate::listen::ensure(&at);
    Ok(())
}

/// One join step. Each re-runs the earlier steps' cheap checks, so a step can
/// be retried alone and nothing is trusted from a previous call.
pub fn step(link: &str, step: Step, folder: Option<&Path>, now: Ms) -> Result<Reply, ApiError> {
    let link = Link::parse(link).ok_or_else(|| err("BadLink", "That isn't a share link. It looks like herdr.dev/join/owner/repo/code."))?;
    let (repo, fold) = session(&link, now)?;
    let mut reply = Reply {
        owner: fold.genesis.owner.clone(),
        name: fold.settings.name.clone(),
        repo: format!("{}/{}", link.repo.owner, link.repo.name),
        branch: fold.genesis.branch.clone(),
        events: fold.items.len(),
        login: None,
        permission: None,
        worktree: None,
        worktree_shown: None,
    };
    if step == Step::Session {
        return Ok(reply);
    }
    let login = signed_in()?;
    reply.login = Some(login.clone());
    if step == Step::Github {
        return Ok(reply);
    }
    let permission = access(&link, &fold, &login)?;
    reply.permission = Some(permission);
    if step == Step::Access {
        return Ok(reply);
    }
    let folder = match folder.map(|f| f.to_string_lossy().into_owned()) {
        Some(f) if f == "~" || f.starts_with("~/") => home_dir().join(f.trim_start_matches('~').trim_start_matches('/')),
        Some(f) => PathBuf::from(f),
        None => default_folder(),
    };
    let path = worktree_path(&folder, &link, &fold.genesis.branch);
    folder_ok(&path, &fold.sid, &fold.genesis.branch)?;
    reply.worktree_shown = Some(tilde(&path));
    reply.worktree = Some(path.clone());
    if step == Step::Folder {
        return Ok(reply);
    }
    check_out(&repo, &fold, &login, permission, &path)?;
    Ok(reply)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_links() {
        let want = Some(Link { repo: GitHubRepo { owner: "deleteme".into(), name: "joindeleterepo".into() }, code: "k7q3m9x4".into() });
        assert_eq!(Link::parse("https://herdr.dev/join/deleteme/joindeleterepo/k7q3m9x4"), want);
        assert_eq!(Link::parse("  herdr.dev/join/deleteme/joindeleterepo/k7q3m9x4/ "), want);
        assert_eq!(Link::parse("herdr.dev/join/k7q3m9x4"), None);
        assert_eq!(Link::parse("https://herdr.dev/join/a/b/K7Q3M9X4"), None);
        assert_eq!(Link::parse("https://herdr.dev/join/a/b/c/k7q3m9x4"), None);
        assert_eq!(Link::parse("https://evil.dev/join/a/b/k7q3m9x4"), None);
    }
}
