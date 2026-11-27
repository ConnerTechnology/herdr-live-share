//! Agents talk to each other (SPEC 2.6, 9.1): an agent's posts, reading what
//! is new, and the guide every agent gets when it starts in a shared checkout.
//! The MCP tools (mcp.rs) and the hooks (claude.rs) are thin layers over this.

use crate::drain;
use crate::git::Checkout;
use crate::ledger::{Body, Fold, ItemKind, Ms, PostKind};
use crate::util::{base32, random32};
use crate::session::{self, Binding};
use crate::view::digest;

/// The longest post, in characters.
pub const MAX_POST: usize = 280;
/// Posts per person per hour.
pub const POSTS_PER_HOUR: usize = 30;
/// The longest answer, in characters.
pub const MAX_ANSWER: usize = 1_000;
/// Questions (to one person or to everyone) per person per hour.
pub const QUESTIONS_PER_HOUR: usize = 10;
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
    TooManyQuestions,
    AnswerTooLong(usize),
    Yourself,
    NotHere(String),
    /// share_answer could not tell which question: the open ones for this person.
    WhichQuestion(Vec<(String, String)>),
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
            Refused::TooManyQuestions => write!(f, "You have asked {QUESTIONS_PER_HOUR} questions in the last hour; wait for answers before asking more."),
            Refused::AnswerTooLong(n) => write!(f, "The answer is {n} characters; keep it under {MAX_ANSWER}."),
            Refused::Yourself => write!(f, "That's your own person; ask someone else in the session."),
            Refused::NotHere(who) => write!(f, "{who} isn't in this shared session."),
            Refused::WhichQuestion(open) if open.is_empty() => write!(f, "There is no open question for you to answer."),
            Refused::WhichQuestion(open) => {
                let list: Vec<String> = open.iter().map(|(id, text)| format!("{id}: {}", crate::view::quote(text))).collect();
                write!(f, "Say which question you are answering, by its id. Open: {}", list.join("; "))
            }
            Refused::Failed(e) => write!(f, "Couldn't record it: {e}"),
        }
    }
}

/// Post for the other agents. Recorded here at once, pushed in the background.
pub fn post(at: &Checkout, b: &Binding, agent: Option<&str>, kind: PostKind, text: &str, now: Ms) -> Result<(), Refused> {
    let text = one_line(text, MAX_POST, Refused::TooLong)?;
    let f = writable(at, b, now)?;
    if in_last_hour(&f, &b.login, now, |body| matches!(body, Body::Post { .. })) >= POSTS_PER_HOUR {
        return Err(Refused::TooMany);
    }
    record(at, b, Body::Post { about: kind, text }, agent, now)
}

/// Ask one person's agents (`to`), or everyone for a second opinion (None).
/// Returns the question's id; the answer arrives like any other news.
pub fn ask(at: &Checkout, b: &Binding, agent: Option<&str>, to: Option<&str>, text: &str, now: Ms) -> Result<String, Refused> {
    let text = one_line(text, MAX_POST, Refused::TooLong)?;
    let f = writable(at, b, now)?;
    let to = match to.map(str::trim) {
        Some(t) if t.eq_ignore_ascii_case(&b.login) => return Err(Refused::Yourself),
        Some(t) => Some(f.members.keys().find(|m| m.eq_ignore_ascii_case(t)).ok_or_else(|| Refused::NotHere(t.to_string()))?.clone()),
        None => None,
    };
    if in_last_hour(&f, &b.login, now, |body| matches!(body, Body::Ask { .. })) >= QUESTIONS_PER_HOUR {
        return Err(Refused::TooManyQuestions);
    }
    let id = new_question_id().map_err(|e| Refused::Failed(e.to_string()))?;
    record(at, b, Body::Ask { id: id.clone(), to, text }, agent, now)?;
    Ok(id)
}

/// Answer a question, named by its id or by (part of) its text, or the only
/// open one for this person. Returns who asked.
pub fn answer(at: &Checkout, b: &Binding, agent: Option<&str>, question: &str, text: &str, background: bool, now: Ms) -> Result<String, Refused> {
    let text = one_line(text, MAX_ANSWER, Refused::AnswerTooLong)?;
    let mut f = writable(at, b, now)?;
    if which_question(&f, &b.login, question).is_err() {
        // Not here yet, or not uniquely: fetch once, then decide.
        session::sync(at, b, std::time::Duration::from_secs(3));
        f = writable(at, b, now)?;
    }
    let id = which_question(&f, &b.login, question)?;
    let asker = f.questions[&id].by.clone();
    record(at, b, Body::Answer { question: id, text, background }, agent, now)?;
    Ok(asker)
}

/// Open questions this person's agents may answer: asked of them, or of everyone, by someone else.
pub fn open_for(f: &Fold, login: &str) -> Vec<(String, String)> {
    f.questions
        .iter()
        .filter(|(_, q)| q.is_open() && q.by != login && q.to.as_deref().is_none_or(|t| t == login))
        .map(|(id, q)| (id.clone(), q.text.clone()))
        .collect()
}

fn which_question(f: &Fold, login: &str, given: &str) -> Result<String, Refused> {
    let given = given.trim();
    if f.questions.contains_key(given) {
        return Ok(given.to_string());
    }
    let open = open_for(f, login);
    let g = given.to_lowercase();
    let quoted: Vec<&(String, String)> = open
        .iter()
        .filter(|(_, t)| !g.is_empty() && (t.to_lowercase().contains(&g) || g.contains(&t.to_lowercase())))
        .collect();
    match (quoted.as_slice(), open.as_slice()) {
        ([(id, _)], _) => Ok(id.clone()),
        (_, [(id, _)]) => Ok(id.clone()),
        _ => Err(Refused::WhichQuestion(open)),
    }
}

/// The session, if this checkout may add to it.
fn writable(at: &Checkout, b: &Binding, now: Ms) -> Result<Fold, Refused> {
    if b.read_only {
        return Err(Refused::ReadOnly);
    }
    let f = session::load(at, b, now).map_err(|e| Refused::Failed(e.to_string()))?;
    if !f.is_open() {
        return Err(Refused::Closed);
    }
    Ok(f)
}

/// Append here at once; push in the background.
fn record(at: &Checkout, b: &Binding, body: Body, agent: Option<&str>, now: Ms) -> Result<(), Refused> {
    session::append(at, b, body, agent, now).map_err(|e| Refused::Failed(e.to_string()))?;
    drain::spawn(at, true);
    Ok(())
}

fn one_line(text: &str, max: usize, too_long: fn(usize) -> Refused) -> Result<String, Refused> {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.is_empty() {
        return Err(Refused::Empty);
    }
    let n = text.chars().count();
    if n > max {
        return Err(too_long(n));
    }
    Ok(text)
}

fn in_last_hour(f: &Fold, login: &str, now: Ms, kind: impl Fn(&Body) -> bool) -> usize {
    f.items
        .iter()
        .filter(|i| i.login.as_deref() == Some(login) && i.at + HOUR > now)
        .filter(|i| matches!(&i.kind, ItemKind::Authored(body) if kind(body)))
        .count()
}

/// "q" and 8 base32 characters.
fn new_question_id() -> std::io::Result<String> {
    Ok(format!("q{}", base32(&random32()?[..5])))
}

/// What is new for this agent since its cursor, after a short fetch (an agent
/// asks right before it decides something). Moves the cursor. None: nothing new.
pub fn read(at: &Checkout, b: &Binding, agent: &str, now: Ms) -> Result<Option<String>, session::Error> {
    session::sync(at, b, std::time::Duration::from_secs(3));
    let f = session::load(at, b, now)?;
    let sid = b.sid();
    let since = session::cursor(at, &sid, agent);
    let text = digest(&f, since.as_ref(), agent, &b.login, now);
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
         - share_post kind \"opinion\": ask everyone for a second opinion when you are unsure; any agent may answer.\n\
         - share_ask: ask one person's agent about something they are working on or have looked into. Keep working; the answer arrives with a tool result or your next prompt.\n\
         - share_answer: answer a question asked of your person, or a second opinion you can help with.\n\
         - share_read: what the others posted since you last looked. News also arrives with your tool results.\n\
         Post only what another agent would act on, in one or two sentences of your own. Never paste your person's prompt, code or secrets: everyone in the session reads posts.\n",
        b.login
    ))
}
