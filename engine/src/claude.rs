//! Claude Code boundary: hook stdin in, hook stdout out.
//! Spike (2026-11-26): `additionalContext` reaches the model for SessionStart,
//! UserPromptSubmit, PreToolUse and PostToolUse, so every reply is silent or context.
//! A hook never fails Claude: whatever happens, it prints valid output or nothing.

use crate::drain;
use crate::git::Checkout;
use crate::ledger::{Body, Ms};
use crate::session::{self, Binding};
use crate::talk;
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
    /// After every tool call: news, between tool calls (SPEC 9.1).
    PostTool,
    Stop,
    SessionEnd,
}

impl HookEvent {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "start" => Self::SessionStart,
            "prompt" => Self::Prompt,
            "pre-edit" => Self::PreEdit,
            "post-tool" => Self::PostTool,
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
            Self::PostTool => "PostToolUse",
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
    /// PreToolUse: the edit's arguments (`file_path`, or `notebook_path`).
    #[serde(default)]
    pub tool_input: serde_json::Value,
    /// Stop: the conversation so far, one JSON object per line.
    #[serde(default)]
    pub transcript_path: Option<PathBuf>,
    /// Stop, in Claude Code releases that send it.
    #[serde(default)]
    pub last_assistant_message: Option<String>,
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
            // Never start a turn while a push after the last one is still rebasing.
            drain::wait_idle(&at, Duration::from_secs(5));
            let _ = drain::notice_push(&at, &b, now_ms());
            let fetch = if event == HookEvent::SessionStart { Duration::from_secs(5) } else { Duration::from_secs(3) };
            catch_up(event, &at, &b, &input.agent(), fetch, now_ms())
        }
        HookEvent::PreEdit => pre_edit(&at, &b, &input, now_ms()),
        HookEvent::PostTool => news(&at, &b, &input, now_ms()),
        HookEvent::Stop => {
            stop(&at, &b, &input, now_ms());
            Reply::silent()
        }
        // The Activity badge is cleared here once Activity exists (step 5).
        HookEvent::SessionEnd => Reply::silent(),
    }
}

/// Before an edit: a heads-up when someone else's agent is on the same file,
/// and "working on" for everyone else (once per file per 10 minutes). Local
/// except for a detached ledger push.
fn pre_edit(at: &Checkout, b: &Binding, input: &HookInput, now: Ms) -> Reply {
    let Some(file) = input.tool_input.get("file_path").or_else(|| input.tool_input.get("notebook_path")).and_then(|p| p.as_str()) else {
        return Reply::silent();
    };
    let file = PathBuf::from(file);
    let abs = if file.is_absolute() { file } else { input.cwd.join(file) };
    let root = at.root.canonicalize().unwrap_or_else(|_| at.root.clone());
    let abs = abs.parent().and_then(|d| d.canonicalize().ok()).map(|d| d.join(abs.file_name().unwrap_or_default())).unwrap_or(abs);
    let Ok(rel) = abs.strip_prefix(&root) else { return Reply::silent() }; // outside the shared checkout
    let path = rel.to_string_lossy().replace('\\', "/");
    fresh_enough(at, b, now);
    let Ok(f) = session::load(at, b, now) else { return Reply::silent() };
    let agent = input.agent();
    let current = f.editing.get(&path);
    let mine = current.is_some_and(|e| e.agent.as_deref() == Some(agent.as_str()));
    if !mine && !b.read_only && f.is_open() && session::append(at, b, Body::WorkingOn { path: path.clone() }, Some(&agent), now).is_ok() {
        drain::spawn(at, true);
    }
    match current {
        Some(e) if e.agent.as_deref() != Some(agent.as_str()) => {
            let who = if e.login == b.login { "Another of your agents".to_string() } else { format!("{}'s agent", e.login) };
            let text = format!(
                "Heads-up from the shared session: {who} is working on {path} ({}). Their changes arrive when they push; keep your edit small or coordinate.",
                ago(now.saturating_sub(e.at))
            );
            Reply {
                out: Some(serde_json::json!({ "hookSpecificOutput": { "hookEventName": "PreToolUse", "additionalContext": text } }).to_string()),
                after: None,
            }
        }
        _ => Reply::silent(),
    }
}

/// Others' "working on" must be current for a heads-up to mean anything: fetch
/// the session's events (small refs only) at most every 10 s per checkout, for
/// at most 2 s. Offline, the heads-up uses what this machine has.
fn fresh_enough(at: &Checkout, b: &Binding, now: Ms) {
    let stamp = at.common_dir.join("herdr-session").join(b.sid().code()).join("pre-edit-fetch");
    let last: Ms = std::fs::read_to_string(&stamp).ok().and_then(|s| s.trim().parse().ok()).unwrap_or(0);
    if now.saturating_sub(last) < 10_000 {
        return;
    }
    let _ = session::write_atomic(&stamp, now.to_string().as_bytes());
    let _ = at.repo().fetch(&b.remote, &session::seen_refspec(&b.sid()), Duration::from_secs(2));
}

/// Between tool calls: what is new since this agent's cursor, from what this
/// machine has (fast, no network). At most every 15 s it also starts a
/// background refresh, so the next tool call has fresher news. Silent for an
/// agent that has not caught up yet: its first prompt gives it everything.
fn news(at: &Checkout, b: &Binding, input: &HookInput, now: Ms) -> Reply {
    refresh_in_background(at, b, now);
    let agent = input.agent();
    let sid = b.sid();
    let Some(since) = session::cursor(at, &sid, &agent) else { return Reply::silent() };
    let Ok(f) = session::load(at, b, now) else { return Reply::silent() };
    let Some(text) = digest(&f, Some(&since), &agent, now) else { return Reply::silent() };
    let term = std::env::var("HC_TERMINAL_ID").ok();
    let (at, heads) = (at.clone(), f.heads.clone());
    Reply {
        out: Some(serde_json::json!({ "hookSpecificOutput": { "hookEventName": "PostToolUse", "additionalContext": text } }).to_string()),
        after: Some(Box::new(move || {
            let _ = session::set_cursor(&at, &sid, &agent, &heads, term.as_deref());
        })),
    }
}

/// Fetch the session (and push ours) in a detached process, at most every 15 s per checkout.
fn refresh_in_background(at: &Checkout, b: &Binding, now: Ms) {
    let stamp = at.common_dir.join("herdr-session").join(b.sid().code()).join("news-fetch");
    let last: Ms = std::fs::read_to_string(&stamp).ok().and_then(|s| s.trim().parse().ok()).unwrap_or(0);
    if now.saturating_sub(last) < 15_000 {
        return;
    }
    let _ = session::write_atomic(&stamp, now.to_string().as_bytes());
    drain::spawn(at, true);
}

/// After a turn: its first line goes to the others, then push (detached).
fn stop(at: &Checkout, b: &Binding, input: &HookInput, now: Ms) {
    if b.read_only {
        return;
    }
    let open = session::load(at, b, now).is_ok_and(|f| f.is_open());
    if open {
        let text = input.last_assistant_message.clone().or_else(|| input.transcript_path.as_deref().and_then(last_reply));
        if let Some(line) = text.as_deref().and_then(first_line) {
            let _ = session::append(at, b, Body::TurnSummary { text: line }, Some(&input.agent()), now);
        }
    }
    // Closed sessions publish nothing, but code pushes stay ordinary git.
    drain::spawn(at, !open);
}

/// The text of the last assistant message in a Claude transcript.
fn last_reply(path: &std::path::Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    text.lines().rev().find_map(|l| {
        let v: serde_json::Value = serde_json::from_str(l).ok()?;
        if v.get("type")?.as_str()? != "assistant" {
            return None;
        }
        let said: Vec<&str> = v.pointer("/message/content")?.as_array()?.iter()
            .filter(|c| c.get("type").and_then(|t| t.as_str()) == Some("text"))
            .filter_map(|c| c.get("text")?.as_str())
            .collect();
        let s = said.join("\n");
        (!s.trim().is_empty()).then_some(s)
    })
}

/// The first line worth reading, without markdown marks, at most 140 characters.
fn first_line(text: &str) -> Option<String> {
    let line = text.lines().map(|l| l.trim().trim_start_matches(['#', '*', '-', '>', ' ']).trim_end_matches('*').trim()).find(|l| !l.is_empty())?;
    Some(if line.chars().count() > 140 { format!("{}…", line.chars().take(139).collect::<String>().trim_end()) } else { line.to_string() })
}

/// Sync, then tell the agent what is new since its cursor (everything recent
/// for a new agent). The cursor moves only after the reply is built.
fn catch_up(event: HookEvent, at: &Checkout, b: &Binding, agent: &str, fetch: Duration, now: Ms) -> Reply {
    let net = session::sync(at, b, fetch);
    let Ok(f) = session::load(at, b, now) else { return Reply::silent() };
    let sid = b.sid();
    let since = session::cursor(at, &sid, agent);
    let mut text = digest(&f, since.as_ref(), agent, now);
    if event == HookEvent::SessionStart {
        if let Some(g) = talk::guide(&f, b) {
            text = Some(match text {
                Some(t) => format!("{t}{g}"),
                None => format!("Shared session \"{}\" (branch {}). Nothing new from the others yet.\n{g}", f.settings.name, f.genesis.branch),
            });
        }
    }
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

// ---------------------------------------------------------------- installing the hooks

/// ~/.claude/settings.json, or $CLAUDE_CONFIG_DIR/settings.json.
pub fn settings_path() -> PathBuf {
    match std::env::var_os("CLAUDE_CONFIG_DIR") {
        Some(d) => PathBuf::from(d).join("settings.json"),
        None => PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".claude/settings.json"),
    }
}

/// (Claude event, matcher, our hook verb).
const HOOKS: [(&str, Option<&str>, &str); 6] = [
    ("SessionStart", None, "start"),
    ("UserPromptSubmit", None, "prompt"),
    ("PreToolUse", Some("Edit|Write|MultiEdit|NotebookEdit"), "pre-edit"),
    ("PostToolUse", None, "post-tool"),
    ("Stop", None, "stop"),
    ("SessionEnd", None, "end"),
];

fn is_ours(hook: &serde_json::Value) -> bool {
    hook.get("command").and_then(|c| c.as_str()).is_some_and(|c| c.contains("herdr-session") && c.contains(" hook "))
}

/// Remove our hooks from a settings document; drop groups and events left empty.
fn strip(doc: &mut serde_json::Value) {
    let Some(hooks) = doc.get_mut("hooks").and_then(|h| h.as_object_mut()) else { return };
    for groups in hooks.values_mut() {
        let Some(list) = groups.as_array_mut() else { continue };
        for g in list.iter_mut() {
            if let Some(hs) = g.get_mut("hooks").and_then(|h| h.as_array_mut()) {
                hs.retain(|h| !is_ours(h));
            }
        }
        list.retain(|g| g.get("hooks").and_then(|h| h.as_array()).is_none_or(|h| !h.is_empty()));
    }
    hooks.retain(|_, v| v.as_array().is_none_or(|a| !a.is_empty()));
    if hooks.is_empty() {
        doc.as_object_mut().unwrap().remove("hooks");
    }
}

/// Our hooks in Claude Code's user settings, pointing at this binary, with the
/// agents' tools pre-approved; then the tools' MCP server registered at user
/// scope. Keeps every other setting and hook. Writes only when something
/// changes, atomically, after saving a one-time backup. Safe to run on every
/// share and join.
pub fn install_hooks() -> std::io::Result<bool> {
    let exe = std::env::current_exe()?;
    let exe = exe.to_string_lossy();
    let cmd = |verb: &str| if exe.contains(' ') { format!("'{exe}' hook {verb}") } else { format!("{exe} hook {verb}") };
    let settings = edit_settings(|doc| {
        strip(doc);
        allow_tools(doc, true);
        let obj = doc.as_object_mut().unwrap();
        let hooks = obj.entry("hooks").or_insert_with(|| serde_json::json!({}));
        let Some(hooks) = hooks.as_object_mut() else { return };
        for (event, matcher, verb) in HOOKS {
            let mut group = serde_json::json!({ "hooks": [{ "type": "command", "command": cmd(verb) }] });
            if let Some(m) = matcher {
                group["matcher"] = m.into();
            }
            let list = hooks.entry(event).or_insert_with(|| serde_json::json!([]));
            if let Some(list) = list.as_array_mut() {
                list.push(group);
            }
        }
    })?;
    Ok(register_tools(&exe) | settings)
}

/// Remove exactly our hooks, the tools' approval and their MCP server.
pub fn uninstall_hooks() -> std::io::Result<bool> {
    let settings = edit_settings(|doc| {
        strip(doc);
        allow_tools(doc, false);
    })?;
    Ok(unregister_tools() | settings)
}

/// The MCP server's name, and the permission rule that approves all its tools.
const TOOLS: &str = "herdr-live-share";
const TOOLS_RULE: &str = "mcp__herdr-live-share";

/// Add or remove our rule in `permissions.allow`, leaving every other rule.
fn allow_tools(doc: &mut serde_json::Value, on: bool) {
    let obj = doc.as_object_mut().unwrap();
    if on {
        let perms = obj.entry("permissions").or_insert_with(|| serde_json::json!({}));
        let Some(perms) = perms.as_object_mut() else { return };
        let allow = perms.entry("allow").or_insert_with(|| serde_json::json!([]));
        if let Some(list) = allow.as_array_mut() {
            if !list.iter().any(|r| r.as_str() == Some(TOOLS_RULE)) {
                list.push(TOOLS_RULE.into());
            }
        }
        return;
    }
    let Some(perms) = obj.get_mut("permissions").and_then(|p| p.as_object_mut()) else { return };
    if let Some(list) = perms.get_mut("allow").and_then(|a| a.as_array_mut()) {
        list.retain(|r| r.as_str() != Some(TOOLS_RULE));
        if list.is_empty() {
            perms.remove("allow");
        }
    }
    if perms.is_empty() {
        obj.remove("permissions");
    }
}

/// Claude Code's own config, where user-scope MCP servers live.
fn claude_json() -> PathBuf {
    match std::env::var_os("CLAUDE_CONFIG_DIR") {
        Some(d) => PathBuf::from(d).join(".claude.json"),
        None => PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".claude.json"),
    }
}

/// The command our MCP server is registered with, if it is.
fn registered_command() -> Option<String> {
    let doc: serde_json::Value = serde_json::from_slice(&std::fs::read(claude_json()).ok()?).ok()?;
    let server = doc.get("mcpServers")?.get(TOOLS)?;
    Some(server.get("command")?.as_str()?.to_string())
}

/// The claude CLI: $HERDR_SESSION_CLAUDE (empty: none), else `claude` on PATH, else ~/.local/bin/claude.
fn claude_cli() -> Option<PathBuf> {
    if let Some(c) = std::env::var_os("HERDR_SESSION_CLAUDE") {
        return (!c.is_empty()).then(|| PathBuf::from(c));
    }
    let on_path = std::env::var_os("PATH").and_then(|p| std::env::split_paths(&p).map(|d| d.join("claude")).find(|c| c.is_file()));
    on_path.or_else(|| {
        let local = PathBuf::from(std::env::var_os("HOME")?).join(".local/bin/claude");
        local.is_file().then_some(local)
    })
}

/// Run the claude CLI with a 20 s limit; true when it exits 0.
fn claude_mcp(args: &[&str]) -> bool {
    let Some(cli) = claude_cli() else { return false };
    let Ok(mut child) = std::process::Command::new(cli)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    else {
        return false;
    };
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) if std::time::Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
        }
    }
}

/// Register `<exe> mcp` as the user-scope MCP server, unless it already is.
/// Without the claude CLI the agents still get news; they just can't post.
fn register_tools(exe: &str) -> bool {
    match registered_command() {
        Some(c) if c == exe => return false,
        Some(_) => {
            claude_mcp(&["mcp", "remove", "--scope", "user", TOOLS]);
        }
        None => {}
    }
    claude_mcp(&["mcp", "add", "--scope", "user", TOOLS, "--", exe, "mcp"])
}

fn unregister_tools() -> bool {
    registered_command().is_some() && claude_mcp(&["mcp", "remove", "--scope", "user", TOOLS])
}

fn edit_settings(f: impl FnOnce(&mut serde_json::Value)) -> std::io::Result<bool> {
    let path = settings_path();
    let old = std::fs::read(&path).ok();
    let mut doc: serde_json::Value = match &old {
        Some(b) if !b.iter().all(u8::is_ascii_whitespace) => serde_json::from_slice(b)
            .map_err(|e| std::io::Error::other(format!("{} is not valid JSON ({e}); left as is", path.display())))?,
        _ => serde_json::json!({}),
    };
    if !doc.is_object() {
        return Err(std::io::Error::other(format!("{} is not a JSON object; left as is", path.display())));
    }
    let before = doc.clone();
    f(&mut doc);
    if doc == before {
        return Ok(false);
    }
    if let Some(b) = &old {
        let backup = path.with_extension("json.herdr-backup");
        if !backup.exists() {
            std::fs::write(&backup, b)?;
        }
    }
    let mut text = serde_json::to_string_pretty(&doc).unwrap();
    text.push('\n');
    session::write_atomic(&path, text.as_bytes())?;
    Ok(true)
}
