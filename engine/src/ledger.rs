//! The session ledger: per-device signed event streams in
//! `refs/herdr/s/<code>/m/<device>`, and the pure, order-independent fold.
#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};

pub struct SessionCode(pub [u8; 5]); // 8 base32 chars
pub struct SessionId(pub [u8; 16]); // hash(genesis)
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Login(pub String);
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct DeviceTag(pub [u8; 8]);
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct AgentId(pub String); // "claude:<session_id>"
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ClaimedTime(pub u64); // self-claimed; never trusted for ordering
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct RepoPath(pub String);
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct TaskId(pub String);
pub struct Oid(pub String);
pub struct Sig(pub Vec<u8>);
pub struct CommitLine { pub oid: Oid, pub subject: String }
pub struct Heads(pub BTreeMap<DeviceTag, u64>);
/// Only `verify` builds it.
pub struct Verified<T>(T);

pub struct Genesis { pub owner: Login, pub branch: String, pub settings: Settings }
pub struct Settings { pub name: String, pub auto_close: bool, pub who_can_join: WhoCanJoin }
/// An empty selection means Anyone; `Only` is never empty.
pub enum WhoCanJoin { AnyoneWithRepoAccess, Only(BTreeSet<Login>) }
pub enum SettingsPatch { Name(String), AutoClose(bool), WhoCanJoin(WhoCanJoin) }

pub struct Event { pub stream: DeviceTag, pub seq: u64, pub at: ClaimedTime, pub login: Login, pub agent: Option<AgentId>, pub body: Body, pub sig: Sig }

pub enum Body {
    Opened(Genesis),
    Closed,
    Reopened,
    SettingsChanged(SettingsPatch),
    Joined,
    Pushed { commits: Vec<CommitLine>, summary: String },
    BranchRewritten { from: Oid, to: Oid },
    Conflict { files: Vec<RepoPath> },
    TaskCreated { id: TaskId, title: String },
    TaskClaimed { id: TaskId },
    TaskDone { id: TaskId },
    WorkingOn { path: RepoPath },
    TurnSummary { text: String },
    Chat { text: String },
    Question { to: Option<Login>, text: String },
    Handoff { to: Option<Login>, text: String },
    Decision { text: String },
}

pub struct Stream { pub device: DeviceTag, pub events: Vec<Event> }

pub enum Status { Open { closes_at: Option<ClaimedTime> }, Closed { at: ClaimedTime, why: CloseWhy } }
pub enum CloseWhy { Owner, Idle }

pub struct Member { pub joined: ClaimedTime, pub hidden: bool, pub last_seen: ClaimedTime }
pub struct Task { pub title: String, pub claimed_by: Option<Login>, pub done: bool }
/// What the Timeline shows; derived kinds have no author and are never written.
pub enum ItemKind { Authored(Body), AutoClosed, EventsHidden { login: Login }, ClosingSoon, CantSync { last_ok: Option<ClaimedTime> } }
pub struct Item { pub at: ClaimedTime, pub login: Option<Login>, pub agent: Option<AgentId>, pub kind: ItemKind }

pub struct Fold {
    pub sid: SessionId,
    pub genesis: Genesis,
    pub settings: Settings,
    pub status: Status,
    pub members: BTreeMap<Login, Member>,
    pub editing: BTreeMap<RepoPath, (Login, AgentId, ClaimedTime)>,
    pub tasks: BTreeMap<TaskId, Task>,
    pub items: Vec<Item>,
    pub heads: Heads,
}

/// Checks signature and seq continuity; the only constructor of `Verified`.
pub fn verify(_s: Stream) -> Option<Verified<Stream>> {
    todo!("not implemented")
}

/// Pure and order-independent: any permutation or duplication of the same streams gives the same Fold.
pub fn fold(_sid: SessionId, _streams: &[Verified<Stream>], _now: ClaimedTime) -> Fold {
    todo!("not implemented")
}
