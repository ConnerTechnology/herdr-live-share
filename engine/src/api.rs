//! What the share dialogs call, one function per verb. main.rs turns each into
//! one JSON envelope. Every function is safe to run twice: a retry after a
//! crash or a double click lands in the same state.

use crate::git::{Checkout, GitError};
use crate::github::{self, Collaborator, GhError, GitHubRepo};
use crate::ledger::{Body, CloseWhy, Fold, Ms, Settings, SettingsPatch, Status, WhoCanJoin};
use crate::session::{self, Binding};
use serde::Serialize;
use std::collections::BTreeSet;
use std::time::Duration;

pub const REMOTE: &str = "origin";
const SYNC_TIMEOUT: Duration = Duration::from_secs(15);

/// An error the dialogs show. `kind` is stable; `message` is for people.
#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct ApiError {
    pub kind: &'static str,
    pub message: String,
}

fn err(kind: &'static str, message: impl Into<String>) -> ApiError {
    ApiError { kind, message: message.into() }
}

impl From<GitError> for ApiError {
    fn from(e: GitError) -> Self {
        err("Git", e.to_string())
    }
}
impl From<GhError> for ApiError {
    fn from(e: GhError) -> Self {
        match e {
            GhError::Missing => err("GhMissing", e.to_string()),
            GhError::NotSignedIn => err("NotSignedIn", e.to_string()),
            GhError::Failed(_) => err("GitHub", e.to_string()),
        }
    }
}
impl From<session::Error> for ApiError {
    fn from(e: session::Error) -> Self {
        match e {
            session::Error::NoSession => err("NoSession", "This session's history can't be found on this machine."),
            e => err("Io", e.to_string()),
        }
    }
}

pub fn checkout(cwd: &std::path::Path) -> Result<Checkout, ApiError> {
    Checkout::discover(cwd).ok_or_else(|| err("NotARepo", "This space's folder is not a git repository on a branch."))
}

// ---------------------------------------------------------------- the remote

#[derive(Debug, Serialize, Clone, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Remote {
    Github { owner: String, name: String },
    Other { url: String },
    None,
}

fn remote_of(at: &Checkout) -> Remote {
    // The configured URL, not an insteadOf rewrite of it.
    match at.repo().run(&["config", "--get", &format!("remote.{REMOTE}.url")]) {
        Ok(url) => {
            let url = url.trim().to_string();
            match GitHubRepo::parse(&url) {
                Some(r) => Remote::Github { owner: r.owner, name: r.name },
                None => Remote::Other { url },
            }
        }
        Err(_) => Remote::None,
    }
}

/// Whether the branch has an upstream on the remote (local: the last fetch's view).
fn on_remote(at: &Checkout) -> bool {
    at.repo().run(&["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"]).is_ok()
}

// ---------------------------------------------------------------- the session

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct Who {
    /// True: anyone with repo access. False: only `logins`.
    pub anyone: bool,
    pub logins: Vec<String>,
}

/// What the Space settings dialog shows.
#[derive(Debug, Serialize)]
pub struct Summary {
    pub code: String,
    pub link: String,
    pub name: String,
    pub open: bool,
    /// "owner" or "idle" when closed.
    pub closed_by: Option<&'static str>,
    pub closes_at: Option<Ms>,
    pub auto_close: bool,
    /// This person's own setting (on this machine), not the session's.
    pub keep_in_sync: bool,
    pub who_can_join: Who,
    pub owner: String,
    pub me: String,
    pub is_owner: bool,
    pub members: Vec<String>,
    /// Whether the last change reached the remote (false: it is saved here and goes with the next sync).
    pub synced: bool,
}

/// `herdr.dev/join/<owner>/<repo>/<code>` (O1 in the spec). Not a secret.
pub fn link(remote: &Remote, code: &str) -> String {
    match remote {
        Remote::Github { owner, name } => format!("https://herdr.dev/join/{owner}/{name}/{code}"),
        _ => format!("herdr-session:{code}"),
    }
}

fn summary(at: &Checkout, b: &Binding, f: &Fold, synced: bool) -> Summary {
    let (open, closed_by, closes_at) = match &f.status {
        Status::Open { closes_at } => (true, None, *closes_at),
        Status::Closed { why: CloseWhy::Owner, .. } => (false, Some("owner"), None),
        Status::Closed { why: CloseWhy::Idle, .. } => (false, Some("idle"), None),
    };
    let who = match &f.settings.who_can_join {
        WhoCanJoin::Anyone => Who { anyone: true, logins: vec![] },
        WhoCanJoin::Only { logins } => Who { anyone: false, logins: logins.iter().cloned().collect() },
    };
    let code = f.sid.code();
    Summary {
        link: link(&remote_of(at), &code),
        code,
        name: f.settings.name.clone(),
        open,
        closed_by,
        closes_at,
        auto_close: f.settings.auto_close,
        keep_in_sync: b.keep_in_sync,
        who_can_join: who,
        owner: f.genesis.owner.clone(),
        me: b.login.clone(),
        is_owner: b.login == f.genesis.owner,
        members: f.members.keys().cloned().collect(),
        synced,
    }
}

/// What the dialog needs to draw: the branch and remote, and the session if
/// this branch is shared. Local only (no network), fast.
#[derive(Debug, Serialize)]
pub struct StatusReply {
    pub branch: String,
    pub remote: Remote,
    pub session: Option<Summary>,
}

pub fn status(at: &Checkout, now: Ms) -> Result<StatusReply, ApiError> {
    let session = match session::binding(at) {
        Some(b) => Some(summary(at, &b, &session::load(at, &b, now)?, true)),
        None => None,
    };
    Ok(StatusReply { branch: at.branch.clone(), remote: remote_of(at), session })
}

/// Turn sharing on. No check for uncommitted or unpushed work: sharing starts
/// at once, and pushes made afterwards become events everyone sees. The branch
/// must be on GitHub, or nobody could join. A branch never shared gets a new
/// session (its link is returned only once the genesis is pushed); a closed one
/// is reopened with the same link; an open one is returned as is.
pub fn start(at: &Checkout, name: &str, now: Ms) -> Result<Summary, ApiError> {
    // The Claude hooks are what make a shared space work for agents: always there (O6).
    let _ = crate::claude::install_hooks();
    if session::binding(at).is_some() {
        // Already shared from here: `change` checks ownership and reopens (a no-op when open).
        return change(at, Change::Sharing(true), now);
    }
    if !matches!(remote_of(at), Remote::Github { .. }) {
        return Err(err("NotGitHub", "Sharing needs the branch on GitHub: this repo's origin is not a GitHub repository."));
    }
    if !on_remote(at) {
        return Err(err("NotPushed", format!("{} isn't on GitHub yet. Push it, then turn sharing on.", at.branch)));
    }
    let login = github::login()?;
    let name = if name.trim().is_empty() { at.branch.clone() } else { name.trim().to_string() };
    let settings = Settings { name, auto_close: false, who_can_join: WhoCanJoin::Anyone };
    session::open(at, REMOTE, &login, settings, now).map_err(|e| match e {
        session::Error::Git(g) => err("Push", format!("Could not create the session on GitHub: {g}")),
        e => e.into(),
    })?;
    let b = session::binding(at).ok_or_else(|| err("Io", "the session was created but not recorded"))?;
    Ok(summary(at, &b, &session::load(at, &b, now)?, true))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    Sharing(bool),
    Name(String),
    AutoClose(bool),
    /// Empty: anyone with repo access.
    WhoCanJoin(BTreeSet<String>),
}

/// Change one session setting. Owner only. Appends an event only if it changes
/// something, then pushes it (a failed push keeps it for the next sync).
pub fn change(at: &Checkout, c: Change, now: Ms) -> Result<Summary, ApiError> {
    let b = session::binding(at).ok_or_else(|| err("NotShared", "This space is not shared."))?;
    let f = session::load(at, &b, now)?;
    if b.login != f.genesis.owner {
        return Err(err("NotOwner", format!("Only {} can change this session's settings.", f.genesis.owner)));
    }
    let body = match c {
        Change::Sharing(true) if !f.is_open() => Some(Body::Reopened),
        Change::Sharing(false) if f.is_open() => Some(Body::Closed),
        Change::Sharing(_) => None,
        Change::Name(n) => {
            let n = n.trim().to_string();
            (!n.is_empty() && n != f.settings.name).then_some(Body::SettingsChanged { patch: SettingsPatch::Name { name: n } })
        }
        Change::AutoClose(on) => (on != f.settings.auto_close).then_some(Body::SettingsChanged { patch: SettingsPatch::AutoClose { on } }),
        Change::WhoCanJoin(logins) => {
            let who = if logins.is_empty() { WhoCanJoin::Anyone } else { WhoCanJoin::Only { logins } };
            (who != f.settings.who_can_join).then_some(Body::SettingsChanged { patch: SettingsPatch::WhoCanJoin { who } })
        }
    };
    let synced = match body {
        Some(body) => {
            session::append(at, &b, body, None, now)?;
            session::sync(at, &b, SYNC_TIMEOUT).last_error.is_none()
        }
        None => true,
    };
    Ok(summary(at, &b, &session::load(at, &b, now)?, synced))
}

/// Keep in sync is each person's own, kept on their machine: anyone in the
/// session sets it, and no event is written.
pub fn set_keep_in_sync(at: &Checkout, on: bool, now: Ms) -> Result<Summary, ApiError> {
    let mut b = session::binding(at).ok_or_else(|| err("NotShared", "This space is not shared."))?;
    b.keep_in_sync = on;
    session::bind(at, &b).map_err(|e| err("Io", e.to_string()))?;
    Ok(summary(at, &b, &session::load(at, &b, now)?, true))
}

/// People with access to the repo, for the Who can join menu.
pub fn collaborators(at: &Checkout) -> Result<Vec<Collaborator>, ApiError> {
    match remote_of(at) {
        Remote::Github { owner, name } => Ok(github::collaborators(&GitHubRepo { owner, name })?),
        _ => Err(err("NotGitHub", "This repo's origin is not on GitHub.")),
    }
}
