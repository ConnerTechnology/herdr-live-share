//! A session as this machine sees it: the device key, which checkout is bound
//! to which session, per-agent cursors, appending our own events, and sync
//! (fetch everyone's streams, push ours). Every write is one CAS ref update or
//! an atomic file replace, so a killed hook leaves no partial state.

use crate::git::{Checkout, GitError};
use crate::ledger::{fold, verify, Body, DeviceKey, DeviceTag, Event, Fold, Genesis, Heads, Ms, SessionId, Settings};
use crate::util::{hex, now_ms, random32, unhex};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// ~/.herdr/session, or $HERDR_SESSION_HOME (tests, and more than one identity on a machine).
pub fn home() -> PathBuf {
    match std::env::var_os("HERDR_SESSION_HOME") {
        Some(h) => PathBuf::from(h),
        None => PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".herdr/session"),
    }
}

/// This machine's key, made on first use.
pub fn device_key() -> std::io::Result<DeviceKey> {
    let path = home().join("device.ed25519");
    if let Ok(text) = std::fs::read_to_string(&path) {
        if let Some(seed) = unhex(text.trim()).and_then(|b| <[u8; 32]>::try_from(b).ok()) {
            return Ok(DeviceKey::from_seed(seed));
        }
    }
    let key = DeviceKey::from_seed(random32()?);
    write_atomic(&path, hex(&key.seed()).as_bytes())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(key)
}

pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

/// Which session a branch of a clone is in. Lives in the clone's shared git
/// dir, so every worktree and every herdr client on this machine sees it.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Binding {
    pub sid: String,
    pub remote: String,
    pub login: String,
    /// Keep in sync, per person: pull with rebase and push after each agent
    /// turn. Lives here, on this person's machine, not in the session.
    #[serde(default = "on")]
    pub keep_in_sync: bool,
}

fn on() -> bool {
    true
}

impl Binding {
    pub fn sid(&self) -> SessionId {
        SessionId::from_hex(&self.sid).expect("binding holds a valid sid")
    }
}

fn state_dir(at: &Checkout) -> PathBuf {
    at.common_dir.join("herdr-session")
}

fn binding_path(at: &Checkout) -> PathBuf {
    state_dir(at).join("bindings").join(format!("{}.json", at.branch.replace('/', "%2F")))
}

pub fn binding(at: &Checkout) -> Option<Binding> {
    let b: Binding = serde_json::from_slice(&std::fs::read(binding_path(at)).ok()?).ok()?;
    SessionId::from_hex(&b.sid)?;
    Some(b)
}

pub fn bind(at: &Checkout, b: &Binding) -> std::io::Result<()> {
    write_atomic(&binding_path(at), &serde_json::to_vec_pretty(b).unwrap())
}

pub fn own_ref(sid: &SessionId, tag: &DeviceTag) -> String {
    format!("refs/herdr/s/{}/m/{}", sid.code(), tag.0)
}

/// The refspec that brings everyone's streams into refs/herdr-seen, which
/// never overwrites our own stream.
pub fn seen_refspec(sid: &SessionId) -> String {
    format!("+refs/herdr/s/{c}/*:refs/herdr-seen/{c}/*", c = sid.code())
}

#[derive(Debug)]
pub enum Error {
    Git(GitError),
    Io(std::io::Error),
    NoSession,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Git(e) => write!(f, "{e}"),
            Error::Io(e) => write!(f, "{e}"),
            Error::NoSession => write!(f, "no session found"),
        }
    }
}

impl From<GitError> for Error {
    fn from(e: GitError) -> Self {
        Error::Git(e)
    }
}
impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

/// Create a session on this checkout's branch: the genesis is seq 0 of our
/// stream. Returns its id once the genesis is pushed (a link must never point
/// to a session nobody can fetch). The core of `start`.
pub fn open(at: &Checkout, remote: &str, login: &str, settings: Settings, now: Ms) -> Result<SessionId, Error> {
    let repo = at.repo();
    let genesis = Genesis {
        repo: repo.remote_url(remote).unwrap_or_default(),
        branch: at.branch.clone(),
        owner: login.to_string(),
        created: now,
        nonce: hex(&random32()?[..8]),
        settings,
    };
    let sid = genesis.session_id();
    let key = device_key()?;
    let event = Event { sid: sid.hex(), seq: 0, at: now, login: login.to_string(), agent: None, body: Body::Opened { genesis } };
    let name = own_ref(&sid, &key.tag(&sid));
    repo.append(&name, None, &key.sign(&event))?;
    repo.push(remote, &format!("{name}:{name}"), Duration::from_secs(30))?;
    bind(at, &Binding { sid: sid.hex(), remote: remote.to_string(), login: login.to_string(), keep_in_sync: true })?;
    Ok(sid)
}

/// Append one event to our stream (local; `sync` or `drain` pushes it).
/// Retries if another local writer moved the ref first.
pub fn append(at: &Checkout, b: &Binding, body: Body, agent: Option<&str>, now: Ms) -> Result<(), Error> {
    let repo = at.repo();
    let sid = b.sid();
    let key = device_key()?;
    let name = own_ref(&sid, &key.tag(&sid));
    for _ in 0..5 {
        let old = repo.ref_oid(&name);
        // A stream we have not written on this machine yet may exist remotely (another worktree, a reinstall).
        let base = old.clone().or_else(|| repo.ref_oid(&name.replace("refs/herdr/s/", "refs/herdr-seen/")));
        let seq = match &base {
            Some(tip) => repo.run(&["rev-list", "--count", "--first-parent", tip])?.trim().parse::<u64>().unwrap_or(0),
            None => 0,
        };
        let event = Event { sid: sid.hex(), seq, at: now, login: b.login.clone(), agent: agent.map(str::to_string), body: body.clone() };
        let blob = key.sign(&event);
        let result = match (&old, &base) {
            (None, Some(seen)) => {
                // Start our local ref from the remote copy, then append.
                repo.run(&["update-ref", &name, seen, "0000000000000000000000000000000000000000"])?;
                repo.append(&name, Some(seen), &blob)
            }
            _ => repo.append(&name, old.as_deref(), &blob),
        };
        match result {
            Ok(_) => {
                touch_tick(&sid);
                return Ok(());
            }
            Err(GitError::Failed(_)) => continue,
            Err(e) => return Err(e.into()),
        }
    }
    Err(Error::Git(GitError::Failed("the stream kept moving; try again".into())))
}

/// Tell watchers (the Activity window) that this machine wrote something.
fn touch_tick(sid: &SessionId) {
    let _ = write_atomic(&home().join("ticks").join(sid.code()), now_ms().to_string().as_bytes());
}

/// The result of talking to the remote.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct NetStatus {
    pub last_ok: Option<Ms>,
    pub last_error: Option<String>,
}

fn net_path(at: &Checkout, sid: &SessionId) -> PathBuf {
    state_dir(at).join(sid.code()).join("net.json")
}

pub fn net_status(at: &Checkout, sid: &SessionId) -> NetStatus {
    std::fs::read(net_path(at, sid)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

/// Fetch everyone's streams and push ours. Idempotent: running it twice is
/// the same as once. Network failures are recorded, not raised: the ledger
/// works from local state.
pub fn sync(at: &Checkout, b: &Binding, timeout: Duration) -> NetStatus {
    let repo = at.repo();
    let sid = b.sid();
    let mut net = net_status(at, &sid);
    let result = (|| -> Result<(), Error> {
        repo.fetch(&b.remote, &seen_refspec(&sid), timeout)?;
        let key = device_key()?;
        let name = own_ref(&sid, &key.tag(&sid));
        if let Some(ours) = repo.ref_oid(&name) {
            let seen = repo.ref_oid(&name.replace("refs/herdr/s/", "refs/herdr-seen/"));
            if seen.as_deref() != Some(ours.as_str()) {
                repo.push(&b.remote, &format!("{name}:{name}"), timeout)?;
            }
        }
        Ok(())
    })();
    match result {
        Ok(()) => {
            net = NetStatus { last_ok: Some(now_ms()), last_error: None };
        }
        Err(e) => net.last_error = Some(e.to_string()),
    }
    let _ = write_atomic(&net_path(at, &sid), &serde_json::to_vec(&net).unwrap());
    net
}

/// Fold every stream this clone has for the session (ours and fetched).
pub fn load(at: &Checkout, b: &Binding, now: Ms) -> Result<Fold, Error> {
    load_repo(&at.repo(), &b.sid(), now)
}

/// Fold every stream a repository (a checkout or a bare clone) has for a session.
pub fn load_repo(repo: &crate::git::Repo, sid: &SessionId, now: Ms) -> Result<Fold, Error> {
    let sid = *sid;
    let mut streams = Vec::new();
    for prefix in [format!("refs/herdr/s/{}/m/", sid.code()), format!("refs/herdr-seen/{}/m/", sid.code())] {
        for (name, _) in repo.refs(&prefix)? {
            let tag = DeviceTag(name.rsplit('/').next().unwrap_or_default().to_string());
            let blobs = repo.read_stream(&name)?;
            streams.push(verify(&sid, &tag, &blobs));
        }
    }
    fold(&sid, &streams, now).ok_or(Error::NoSession)
}

/// Where an agent is in the ledger.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Cursor {
    pub heads: BTreeMap<String, u64>,
    pub term: Option<String>,
}

fn cursor_path(at: &Checkout, sid: &SessionId, agent: &str) -> PathBuf {
    let safe: String = agent.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' }).collect();
    state_dir(at).join(sid.code()).join("cursors").join(format!("{safe}.json"))
}

/// None: this agent has never caught up (a new agent gets the full digest).
pub fn cursor(at: &Checkout, sid: &SessionId, agent: &str) -> Option<Heads> {
    let c: Cursor = serde_json::from_slice(&std::fs::read(cursor_path(at, sid, agent)).ok()?).ok()?;
    Some(c.heads.into_iter().map(|(t, s)| (DeviceTag(t), s)).collect())
}

pub fn set_cursor(at: &Checkout, sid: &SessionId, agent: &str, heads: &Heads, term: Option<&str>) -> std::io::Result<()> {
    let c = Cursor { heads: heads.iter().map(|(t, s)| (t.0.clone(), *s)).collect(), term: term.map(str::to_string) };
    write_atomic(&cursor_path(at, sid, agent), &serde_json::to_vec(&c).unwrap())
}
