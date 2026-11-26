//! Public surface. main.rs maps each fn to one verb and one JSON envelope.
//! Every mutating fn takes an OperationId so retries converge.
#![allow(dead_code)]

use crate::ledger::{SessionCode, Settings, Status, WhoCanJoin};
use std::path::PathBuf;
use std::time::Duration;

pub struct Checkout { pub root: PathBuf, pub common_dir: PathBuf, pub branch: String }
pub struct OperationId(pub [u8; 16]);

pub enum Upstream { Tracking, NotSet, Gone }
pub enum RemoteKind { GitHub { owner: String, name: String }, Other { origin: String } }
pub struct Preflight { pub branch: String, pub changed_files: u32, pub unpushed_commits: u32, pub upstream: Upstream, pub remote: RemoteKind }
pub struct Publish { pub commit_all: bool }
pub enum Capability { Read, ReadWrite }
pub struct ShareSummary { pub code: SessionCode, pub link: String, pub settings: Settings, pub status: Status, pub capability: Capability }
pub enum Change { Sharing(bool), Name(String), AutoClose(bool), WhoCanJoin(WhoCanJoin) }
pub struct Collaborator { pub login: String, pub capability: Capability }
pub enum JoinStep { Session, GitHub, Access, Folder, Branch, Finish }
pub enum JoinFail { SessionNotFound, NotSignedIn, NoAccess, FolderUnusable { reason: String }, BranchGone }
pub enum StepResult { Pass, Fail(JoinFail) }
pub struct JoinOp { pub op: OperationId, pub link: String, pub folder: PathBuf }
pub enum Budget { LocalOnly, Network(Duration) }
pub struct Synced;
pub enum Error { NotARepo, NotOwner, Closed, Git(String), GitHub(String), Io(String), Busy }

pub fn preflight(_at: &Checkout) -> Result<Preflight, Error> { todo!("not implemented") }
/// Optional commit of everything, then push -u.
pub fn publish(_at: &Checkout, _op: OperationId, _what: Publish) -> Result<Preflight, Error> { todo!("not implemented") }
/// Returns the link only after the genesis push succeeds. Reopens if bound and closed.
pub fn start(_at: &Checkout, _op: OperationId, _name: String) -> Result<ShareSummary, Error> { todo!("not implemented") }
pub fn change(_at: &Checkout, _op: OperationId, _c: Change) -> Result<ShareSummary, Error> { todo!("not implemented") }
/// Local only, < 50 ms.
pub fn status(_at: &Checkout) -> Result<Option<ShareSummary>, Error> { todo!("not implemented") }
pub fn collaborators(_at: &Checkout) -> Result<Vec<Collaborator>, Error> { todo!("not implemented") }
pub fn join_step(_op: &JoinOp, _step: JoinStep) -> Result<StepResult, Error> { todo!("not implemented") }
/// Idempotent; the only way state moves.
pub fn sync(_at: &Checkout, _budget: Budget) -> Result<Synced, Error> { todo!("not implemented") }
