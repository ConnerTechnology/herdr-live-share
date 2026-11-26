//! The session ledger: one signed event stream per device, stored as a commit
//! chain in `refs/herdr/s/<code>/m/<device-tag>`, and `fold`, which turns any
//! set of verified streams into the session state. `fold` is pure and
//! order-independent: the same streams in any order, duplicated or with stale
//! copies, give the same `Fold` on every machine.

use crate::util::{base32, hex, sha256, unhex};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Milliseconds since the Unix epoch, as claimed by the author's clock. Used
/// for display and ordering only, never for trust.
pub type Ms = u64;

pub const IDLE_CLOSE: Ms = 14 * 24 * 3_600_000;
pub const CLOSING_SOON: Ms = 48 * 3_600_000;
pub const EDITING_FRESH: Ms = 10 * 60_000;

// ---------------------------------------------------------------- identities

/// hash(genesis)[..16]. Its first 5 bytes are the session code in refs and links.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct SessionId(pub [u8; 16]);

impl SessionId {
    pub fn hex(&self) -> String {
        hex(&self.0)
    }
    pub fn from_hex(s: &str) -> Option<Self> {
        Some(SessionId(unhex(s)?.try_into().ok()?))
    }
    /// 8 base32 characters: the ref segment and the end of the share link.
    pub fn code(&self) -> String {
        base32(&self.0[..5])
    }
}

/// hash(sid ‖ device public key)[..8], hex. One stream per device per session.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct DeviceTag(pub String);

impl DeviceTag {
    pub fn of(sid: &SessionId, pk: &[u8; 32]) -> Self {
        let mut b = sid.0.to_vec();
        b.extend_from_slice(pk);
        DeviceTag(hex(&sha256(&b)[..8]))
    }
}

/// The last seq seen per stream: a vector cursor.
pub type Heads = BTreeMap<DeviceTag, u64>;

// ---------------------------------------------------------------- events

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub struct Settings {
    pub name: String,
    pub auto_close: bool,
    pub who_can_join: WhoCanJoin,
}

/// An empty `Only` means Anyone: `fold` normalizes it.
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
#[serde(tag = "who", rename_all = "snake_case")]
pub enum WhoCanJoin {
    Anyone,
    Only { logins: BTreeSet<String> },
}

impl WhoCanJoin {
    fn normalized(self) -> Self {
        match self {
            WhoCanJoin::Only { logins } if logins.is_empty() => WhoCanJoin::Anyone,
            w => w,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub struct Genesis {
    pub repo: String,
    pub branch: String,
    pub owner: String,
    pub created: Ms,
    /// Random, so two sessions on the same branch get different ids.
    pub nonce: String,
    pub settings: Settings,
}

impl Genesis {
    pub fn session_id(&self) -> SessionId {
        let json = serde_json::to_vec(self).expect("genesis serializes");
        SessionId(sha256(&json)[..16].try_into().unwrap())
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
#[serde(tag = "set", rename_all = "snake_case")]
pub enum SettingsPatch {
    Name { name: String },
    AutoClose { on: bool },
    WhoCanJoin { who: WhoCanJoin },
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub struct CommitLine {
    pub oid: String,
    pub subject: String,
}

/// What an event says. A release may add variants and fields, never change or
/// remove one: an older engine decodes a variant it does not know as `Unknown`
/// and shows nothing for it.
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Body {
    Opened { genesis: Genesis },
    Closed,
    Reopened,
    SettingsChanged { patch: SettingsPatch },
    Joined,
    Pushed { commits: Vec<CommitLine>, summary: String },
    BranchRewritten { from: String, to: String },
    Conflict { files: Vec<String> },
    TaskCreated { id: String, title: String },
    TaskClaimed { id: String },
    TaskDone { id: String },
    WorkingOn { path: String },
    TurnSummary { text: String },
    Chat { text: String },
    Question { to: Option<String>, text: String },
    Handoff { to: Option<String>, text: String },
    Decision { text: String },
    /// Never written: a kind from a newer release.
    #[serde(skip)]
    Unknown { kind: String },
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub struct Event {
    pub sid: String,
    pub seq: u64,
    pub at: Ms,
    pub login: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    pub body: Body,
}

/// The event as decoded, with its raw body for kinds this release does not know.
#[derive(Deserialize)]
struct WireEvent {
    sid: String,
    seq: u64,
    at: Ms,
    login: String,
    #[serde(default)]
    agent: Option<String>,
    body: serde_json::Value,
}

/// An event with its signature. `bytes` are exactly what was signed and stored.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Signed {
    pub pk: [u8; 32],
    pub bytes: Vec<u8>,
    pub event: Event,
}

impl Signed {
    /// The blob stored in git: the signed JSON line, then "pk sig".
    pub fn to_blob(&self, sig: &[u8; 64]) -> Vec<u8> {
        let mut b = self.bytes.clone();
        b.push(b'\n');
        b.extend_from_slice(format!("{} {}\n", hex(&self.pk), hex(sig)).as_bytes());
        b
    }
}

/// Decode and check one stored blob. None if it is malformed or the signature fails.
pub fn decode_blob(blob: &[u8]) -> Option<Signed> {
    let text = std::str::from_utf8(blob).ok()?;
    let (line, rest) = text.split_once('\n')?;
    let (pk_hex, sig_hex) = rest.trim_end().split_once(' ')?;
    let pk: [u8; 32] = unhex(pk_hex)?.try_into().ok()?;
    let sig: [u8; 64] = unhex(sig_hex)?.try_into().ok()?;
    VerifyingKey::from_bytes(&pk).ok()?.verify(line.as_bytes(), &Signature::from_bytes(&sig)).ok()?;
    let w: WireEvent = serde_json::from_str(line).ok()?;
    let body = match serde_json::from_value::<Body>(w.body.clone()) {
        Ok(b) => b,
        Err(_) => Body::Unknown { kind: w.body.get("kind").and_then(|k| k.as_str()).unwrap_or("").to_string() },
    };
    let event = Event { sid: w.sid, seq: w.seq, at: w.at, login: w.login, agent: w.agent, body };
    Some(Signed { pk, bytes: line.as_bytes().to_vec(), event })
}

/// This machine's signing key. One per machine, shared by all its sessions.
pub struct DeviceKey(SigningKey);

impl DeviceKey {
    pub fn from_seed(seed: [u8; 32]) -> Self {
        DeviceKey(SigningKey::from_bytes(&seed))
    }
    pub fn seed(&self) -> [u8; 32] {
        self.0.to_bytes()
    }
    pub fn pk(&self) -> [u8; 32] {
        self.0.verifying_key().to_bytes()
    }
    pub fn tag(&self, sid: &SessionId) -> DeviceTag {
        DeviceTag::of(sid, &self.pk())
    }
    /// Sign an event; returns the blob to store.
    pub fn sign(&self, event: &Event) -> Vec<u8> {
        let bytes = serde_json::to_vec(event).expect("event serializes");
        let sig = self.0.sign(&bytes).to_bytes();
        Signed { pk: self.pk(), bytes, event: event.clone() }.to_blob(&sig)
    }
}

// ---------------------------------------------------------------- streams

/// One device's events, seq 0.. in order, all checked. Only `verify` builds it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Verified {
    tag: DeviceTag,
    events: Vec<Signed>,
}

impl Verified {
    pub fn tag(&self) -> &DeviceTag {
        &self.tag
    }
    pub fn events(&self) -> &[Signed] {
        &self.events
    }
}

/// Keeps the longest valid prefix of a stream's blobs (seq 0 first): every
/// event decodes, is signed by one key whose tag is `tag`, names `sid`, and
/// has seq equal to its position. Anything after the first bad event is dropped.
pub fn verify(sid: &SessionId, tag: &DeviceTag, blobs: &[Vec<u8>]) -> Verified {
    let sid_hex = sid.hex();
    let mut events = Vec::new();
    for (i, blob) in blobs.iter().enumerate() {
        let Some(s) = decode_blob(blob) else { break };
        let ok = DeviceTag::of(sid, &s.pk) == *tag && s.event.sid == sid_hex && s.event.seq == i as u64;
        if !ok {
            break;
        }
        events.push(s);
    }
    Verified { tag: tag.clone(), events }
}

/// Several copies of one device's stream (ours, the remote's, a stale one) to
/// one: the longest when they are all prefixes of each other, else their
/// common prefix (a forked stream is cut back to what everyone agrees on).
/// A function of the set of copies, so the order they arrive in never matters.
fn merge_copies(copies: &[&Verified]) -> Vec<Signed> {
    let longest = copies.iter().max_by_key(|c| c.events.len()).unwrap();
    let chain = copies.iter().all(|c| longest.events.starts_with(&c.events));
    if chain {
        return longest.events.clone();
    }
    let mut n = 0;
    while copies.iter().all(|c| c.events.get(n).is_some_and(|e| *e == copies[0].events[n])) {
        n += 1;
    }
    copies[0].events[..n].to_vec()
}

// ---------------------------------------------------------------- fold

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Status {
    Open { closes_at: Option<Ms> },
    Closed { at: Ms, why: CloseWhy },
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CloseWhy {
    Owner,
    Idle,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Member {
    pub joined: Ms,
    pub last_seen: Ms,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Task {
    pub title: String,
    pub by: String,
    pub claimed_by: Option<String>,
    pub done: bool,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Editing {
    pub login: String,
    pub agent: Option<String>,
    pub at: Ms,
}

/// What the Timeline shows. Derived kinds have no author and are never written.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ItemKind {
    Authored(Body),
    AutoClosed,
    ClosingSoon { closes_at: Ms },
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Item {
    pub at: Ms,
    /// Where an authored item lives; None for derived ones.
    pub pos: Option<(DeviceTag, u64)>,
    pub login: Option<String>,
    pub agent: Option<String>,
    pub kind: ItemKind,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Fold {
    pub sid: SessionId,
    pub genesis: Genesis,
    pub settings: Settings,
    pub status: Status,
    pub members: BTreeMap<String, Member>,
    /// Newest WorkingOn per path, younger than 10 minutes at `now`.
    pub editing: BTreeMap<String, Editing>,
    pub tasks: BTreeMap<String, Task>,
    /// Oldest first, in (at, device, seq) order.
    pub items: Vec<Item>,
    pub heads: Heads,
    pub last_activity: Ms,
}

impl Fold {
    pub fn is_open(&self) -> bool {
        matches!(self.status, Status::Open { .. })
    }
}

/// The session state from every verified stream we have. None when no stream
/// holds this session's genesis (seq 0, `Opened`, hashing to `sid`).
pub fn fold(sid: &SessionId, streams: &[Verified], now: Ms) -> Option<Fold> {
    // One stream per device.
    let mut by_tag: BTreeMap<&DeviceTag, Vec<&Verified>> = BTreeMap::new();
    for s in streams {
        by_tag.entry(&s.tag).or_default().push(s);
    }
    let merged: BTreeMap<DeviceTag, Vec<Signed>> =
        by_tag.into_iter().map(|(t, copies)| (t.clone(), merge_copies(&copies))).collect();

    let genesis = merged.values().find_map(|evs| match &evs.first()?.event.body {
        Body::Opened { genesis } if genesis.session_id() == *sid => Some(genesis.clone()),
        _ => None,
    })?;
    let owner = genesis.owner.clone();

    let heads: Heads =
        merged.iter().filter(|(_, e)| !e.is_empty()).map(|(t, e)| (t.clone(), e.len() as u64 - 1)).collect();

    let mut all: Vec<(&DeviceTag, &Signed)> = merged.iter().flat_map(|(t, evs)| evs.iter().map(move |e| (t, e))).collect();
    all.sort_by(|a, b| (a.1.event.at, a.0, a.1.event.seq).cmp(&(b.1.event.at, b.0, b.1.event.seq)));

    let mut settings = genesis.settings.clone();
    settings.who_can_join = settings.who_can_join.normalized();
    let mut status = Status::Open { closes_at: None };
    let mut last = genesis.created;
    let mut members: BTreeMap<String, Member> = BTreeMap::new();
    members.insert(owner.clone(), Member { joined: genesis.created, last_seen: genesis.created });
    let mut editing: BTreeMap<String, Editing> = BTreeMap::new();
    let mut tasks: BTreeMap<String, Task> = BTreeMap::new();
    let mut items = Vec::new();

    for (tag, s) in all {
        let e = &s.event;
        let open = matches!(status, Status::Open { .. });
        if open && settings.auto_close && e.at > last + IDLE_CLOSE {
            status = Status::Closed { at: last + IDLE_CLOSE, why: CloseWhy::Idle };
            items.push(Item { at: last + IDLE_CLOSE, pos: None, login: None, agent: None, kind: ItemKind::AutoClosed });
        }
        let by_owner = e.login == owner;
        match &e.body {
            Body::Opened { .. } => {}
            Body::Closed if by_owner => status = Status::Closed { at: e.at, why: CloseWhy::Owner },
            Body::Reopened if by_owner => status = Status::Open { closes_at: None },
            Body::SettingsChanged { patch } if by_owner => match patch.clone() {
                SettingsPatch::Name { name } => settings.name = name,
                SettingsPatch::AutoClose { on } => settings.auto_close = on,
                SettingsPatch::WhoCanJoin { who } => settings.who_can_join = who.normalized(),
            },
            Body::Closed | Body::Reopened | Body::SettingsChanged { .. } => continue, // not the owner: ignored
            Body::Joined => {
                members.entry(e.login.clone()).or_insert(Member { joined: e.at, last_seen: e.at });
            }
            Body::TaskCreated { id, title } => {
                tasks.entry(id.clone()).or_insert(Task { title: title.clone(), by: e.login.clone(), claimed_by: None, done: false });
            }
            Body::TaskClaimed { id } => {
                if let Some(t) = tasks.get_mut(id) {
                    t.claimed_by = Some(e.login.clone());
                }
            }
            Body::TaskDone { id } => {
                if let Some(t) = tasks.get_mut(id) {
                    t.done = true;
                }
            }
            Body::WorkingOn { path } => {
                editing.insert(path.clone(), Editing { login: e.login.clone(), agent: e.agent.clone(), at: e.at });
            }
            _ => {}
        }
        if let Some(m) = members.get_mut(&e.login) {
            m.last_seen = m.last_seen.max(e.at);
        }
        if matches!(status, Status::Open { .. }) {
            last = last.max(e.at);
        }
        if !matches!(e.body, Body::Unknown { .. }) {
            items.push(Item {
                at: e.at,
                pos: Some((tag.clone(), e.seq)),
                login: Some(e.login.clone()),
                agent: e.agent.clone(),
                kind: ItemKind::Authored(e.body.clone()),
            });
        }
    }

    // Nothing runs at the deadline: whoever reads evaluates it against `now`.
    if matches!(status, Status::Open { .. }) && settings.auto_close {
        let closes = last + IDLE_CLOSE;
        if now >= closes {
            status = Status::Closed { at: closes, why: CloseWhy::Idle };
            items.push(Item { at: closes, pos: None, login: None, agent: None, kind: ItemKind::AutoClosed });
        } else {
            status = Status::Open { closes_at: Some(closes) };
            if now + CLOSING_SOON >= closes {
                let at = closes - CLOSING_SOON;
                items.push(Item { at, pos: None, login: None, agent: None, kind: ItemKind::ClosingSoon { closes_at: closes } });
            }
        }
    }
    editing.retain(|_, e| now.saturating_sub(e.at) < EDITING_FRESH);

    Some(Fold { sid: *sid, genesis, settings, status, members, editing, tasks, items, heads, last_activity: last })
}

/// Whether an item is after a cursor.
pub fn is_new(item: &Item, since: &Heads) -> bool {
    match &item.pos {
        Some((tag, seq)) => since.get(tag).is_none_or(|s| seq > s),
        None => false,
    }
}

#[cfg(test)]
mod tests;
