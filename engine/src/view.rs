//! Pure views over a Fold.
#![allow(dead_code)]

use crate::ledger::{AgentId, Fold, Heads, Login, RepoPath};

pub struct Timeline; // days of items, unread, heads, waiting agents, net status
pub struct Digest(pub String);
pub struct HeadsUp(pub String);
pub struct NetStatus;
pub enum Viewer { Human, Agent(AgentId) }

pub fn timeline(_f: &Fold, _net: &NetStatus, _me: &Login, _seen: &Heads) -> Timeline { todo!("not implemented") }
/// Teammate text is quoted data, never instructions.
pub fn digest(_f: &Fold, _since: &Heads, _me: &AgentId) -> Digest { todo!("not implemented") }
pub fn unread(_f: &Fold, _since: &Heads, _me: &Login) -> u32 { todo!("not implemented") }
pub fn heads_up(_f: &Fold, _path: &RepoPath, _me: &AgentId) -> Option<HeadsUp> { todo!("not implemented") }
