//! Claude Code boundary: hook stdin in, hook stdout out.
//! Spike (2026-11-26): `additionalContext` reaches the model for SessionStart,
//! UserPromptSubmit and PreToolUse, so every reply is silent or context.
//! A hook never fails Claude: whatever happens, it prints valid output or nothing.

use crate::git::Checkout;
use crate::ledger::Ms;
use crate::session::{self, Binding};
use crate::util::{ago, now_ms};
use crate::view::digest;
use serde::Deserialize;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HookEvent {
    SessionStart,
    Prompt,
    PreEdit,
    Stop,
    SessionEnd,
}

impl HookEvent {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "start" => Self::SessionStart,
            "prompt" => Self::Prompt,
            "pre-edit" => Self::PreEdit,
            "stop" => Self::Stop,
            "end" => Self::SessionEnd,
            _ => return None,
        })
    }

    /// The `hookEventName` Claude expects back.
    pub fn claude_name(self) -> &'static str {
        match self {
            Self::SessionStart => "SessionStart",
            Self::Prompt => "UserPromptSubmit",
            Self::PreEdit => "PreToolUse",
            Self::Stop => "Stop",
            Self::SessionEnd => "SessionEnd",
        }
    }
}

/// The parts of Claude's hook stdin we use.
#[derive(Deserialize, Debug)]
pub struct HookInput {
    pub session_id: String,
    pub cwd: PathBuf,
}

impl HookInput {
    /// One Claude conversation is one agent with its own cursor.
    pub fn agent(&self) -> String {
        format!("claude:{}", self.session_id)
    }
}

/// What a hook prints, and what to record once it has been printed.
pub struct Reply {
    pub out: Option<String>,
    pub after: Option<Box<dyn FnOnce()>>,
}

impl Reply {
    pub fn silent() -> Self {
        Reply { out: None, after: None }
    }
}

/// Runs one hook. The caller prints `out`, flushes, then runs `after`
/// (a cursor moves only once the agent has the text; a repeat is acceptable, a gap is not).
pub fn hook(event: HookEvent, stdin: &str) -> Reply {
    let Ok(input) = serde_json::from_str::<HookInput>(stdin) else { return Reply::silent() };
    let Some(at) = Checkout::discover(&input.cwd) else { return Reply::silent() };
    let Some(b) = session::binding(&at) else { return Reply::silent() }; // not a shared checkout: silent, fast
    match event {
        HookEvent::SessionStart | HookEvent::Prompt => {
            let fetch = if event == HookEvent::SessionStart { Duration::from_secs(5) } else { Duration::from_secs(3) };
            catch_up(event, &at, &b, &input.agent(), fetch, now_ms())
        }
        // Later steps: heads-up before edits, turn summaries and push, badge clear.
        HookEvent::PreEdit | HookEvent::Stop | HookEvent::SessionEnd => Reply::silent(),
    }
}

/// Sync, then tell the agent what is new since its cursor (everything recent
/// for a new agent). The cursor moves only after the reply is built.
fn catch_up(event: HookEvent, at: &Checkout, b: &Binding, agent: &str, fetch: Duration, now: Ms) -> Reply {
    let net = session::sync(at, b, fetch);
    let Ok(f) = session::load(at, b, now) else { return Reply::silent() };
    let sid = b.sid();
    let since = session::cursor(at, &sid, agent);
    let mut text = digest(&f, since.as_ref(), agent, now);
    if net.last_error.is_some() {
        let when = net.last_ok.map(|t| format!("last fetched {}", ago(now.saturating_sub(t)))).unwrap_or_else(|| "never fetched".into());
        let note = format!("Can't reach the remote ({when}); this is what this machine has.\n");
        text = text.map(|t| format!("{note}{t}"));
    }
    let term = std::env::var("HC_TERMINAL_ID").ok();
    let (at, agent, heads) = (at.clone(), agent.to_string(), f.heads.clone());
    Reply {
        out: text.map(|t| {
            serde_json::json!({ "hookSpecificOutput": { "hookEventName": event.claude_name(), "additionalContext": t } })
                .to_string()
        }),
        after: Some(Box::new(move || {
            let _ = session::set_cursor(&at, &sid, &agent, &heads, term.as_deref());
        })),
    }
}
