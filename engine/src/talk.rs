//! Agents talk to each other (SPEC 2.6, 9.1): an agent's posts, reading what
//! is new, and the guide every agent gets when it starts in a shared checkout.
//! The MCP tools (mcp.rs) and the hooks (claude.rs) are thin layers over this.

use crate::drain;
use crate::git::Checkout;
use crate::ledger::{Body, Fold, ItemKind, Ms, PostKind};
use crate::session::{self, Binding};
use crate::view::digest;

/// The longest post, in characters.
pub const MAX_POST: usize = 280;
/// Posts per person per hour.
pub const POSTS_PER_HOUR: usize = 30;
const HOUR: Ms = 3_600_000;

/// Why a post was not made, in words for the agent.
#[derive(Debug, PartialEq, Eq)]
pub enum Refused {
    NotShared,
    ReadOnly,
    Closed,
    Empty,
    TooLong(usize),
    TooMany,
    Failed(String),
}

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refused::NotShared => write!(f, "This folder isn't in a shared space, so there is no one to tell."),
            Refused::ReadOnly => write!(f, "Your person joined with read access, so nothing from this checkout is shared."),
            Refused::Closed => write!(f, "The shared session is closed, so nothing new is shared."),
            Refused::Empty => write!(f, "The post is empty."),
            Refused::TooLong(n) => write!(f, "The post is {n} characters; keep it under {MAX_POST}."),
            Refused::TooMany => write!(f, "You have posted {POSTS_PER_HOUR} times in the last hour; post again later, and only what others would act on."),
            Refused::Failed(e) => write!(f, "Couldn't record the post: {e}"),
        }
    }
}

/// Post for the other agents. Recorded here at once, pushed in the background.
pub fn post(at: &Checkout, b: &Binding, agent: Option<&str>, kind: PostKind, text: &str, now: Ms) -> Result<(), Refused> {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.is_empty() {
        return Err(Refused::Empty);
    }
    let n = text.chars().count();
    if n > MAX_POST {
        return Err(Refused::TooLong(n));
    }
    if b.read_only {
        return Err(Refused::ReadOnly);
    }
    let f = session::load(at, b, now).map_err(|e| Refused::Failed(e.to_string()))?;
    if !f.is_open() {
        return Err(Refused::Closed);
    }
    if posts_in_last_hour(&f, &b.login, now) >= POSTS_PER_HOUR {
        return Err(Refused::TooMany);
    }
    session::append(at, b, Body::Post { about: kind, text }, agent, now).map_err(|e| Refused::Failed(e.to_string()))?;
    drain::spawn(at, true);
    Ok(())
}

fn posts_in_last_hour(f: &Fold, login: &str, now: Ms) -> usize {
    f.items
        .iter()
        .filter(|i| i.login.as_deref() == Some(login) && i.at + HOUR > now)
        .filter(|i| matches!(i.kind, ItemKind::Authored(Body::Post { .. })))
        .count()
}

/// What is new for this agent since its cursor, after a short fetch (an agent
/// asks right before it decides something). Moves the cursor. None: nothing new.
pub fn read(at: &Checkout, b: &Binding, agent: &str, now: Ms) -> Result<Option<String>, session::Error> {
    session::sync(at, b, std::time::Duration::from_secs(3));
    let f = session::load(at, b, now)?;
    let sid = b.sid();
    let since = session::cursor(at, &sid, agent);
    let text = digest(&f, since.as_ref(), agent, now);
    let term = std::env::var("HC_TERMINAL_ID").ok();
    session::set_cursor(at, &sid, agent, &f.heads, term.as_deref())?;
    Ok(text)
}

/// How to talk to the others, for an agent starting in a shared checkout it can write to.
pub fn guide(f: &Fold, b: &Binding) -> Option<String> {
    if b.read_only || !f.is_open() {
        return None;
    }
    let others: Vec<&str> = f.members.keys().map(String::as_str).filter(|l| *l != b.login).collect();
    let who = if others.is_empty() { "No one else has joined yet; they will see what you post when they do.".to_string() } else { format!("Also here: {}.", others.join(", ")) };
    Some(format!(
        "You are one of the agents in this shared session, working for {}. {who}\n\
         The herdr-live-share tools let the agents talk:\n\
         - share_post kind \"working\": what you are about to change, when it could overlap with someone else's work.\n\
         - share_post kind \"finding\": something you learned that would save another agent time.\n\
         - share_read: what the others posted since you last looked. News also arrives with your tool results.\n\
         Post only what another agent would act on, in one or two sentences of your own. Never paste your person's prompt, code or secrets: everyone in the session reads posts.\n",
        b.login
    ))
}
