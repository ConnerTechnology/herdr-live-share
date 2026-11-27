//! `herdr-session mcp`: the tools Claude agents use to talk to each other
//! (SPEC 9.1). An MCP server over stdio, one JSON-RPC message per line.
//! Claude Code starts it in the project folder and passes the conversation's
//! id in CLAUDE_CODE_SESSION_ID (checked 2026-11-26, Claude Code 2.1.332).

use crate::git::Checkout;
use crate::ledger::PostKind;
use crate::session;
use crate::talk::{self, Refused, MAX_ANSWER, MAX_POST};
use crate::util::now_ms;
use serde_json::{json, Value};
use std::io::{BufRead, Write};

/// Serve until stdin closes.
pub fn serve(input: impl BufRead, mut out: impl Write) {
    for line in input.lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let Ok(msg) = serde_json::from_str::<Value>(&line) else { continue };
        if let Some(reply) = handle(&msg) {
            if writeln!(out, "{reply}").and_then(|_| out.flush()).is_err() {
                break;
            }
        }
    }
}

/// The reply to one message; None for notifications.
fn handle(msg: &Value) -> Option<Value> {
    let id = msg.get("id")?.clone();
    let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
    let result = match method {
        "initialize" => {
            let version = msg.pointer("/params/protocolVersion").and_then(Value::as_str).unwrap_or("2025-06-18");
            json!({
                "protocolVersion": version,
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "herdr-live-share", "version": env!("CARGO_PKG_VERSION") },
            })
        }
        "ping" => json!({}),
        "tools/list" => json!({ "tools": tools() }),
        "tools/call" => {
            let name = msg.pointer("/params/name").and_then(Value::as_str).unwrap_or("");
            let args = msg.pointer("/params/arguments").cloned().unwrap_or(json!({}));
            let (text, is_error) = match call(name, &args) {
                Ok(t) => (t, false),
                Err(t) => (t, true),
            };
            json!({ "content": [{ "type": "text", "text": text }], "isError": is_error })
        }
        _ => return Some(json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32601, "message": format!("unknown method {method}") } })),
    };
    Some(json!({ "jsonrpc": "2.0", "id": id, "result": result }))
}

fn tools() -> Value {
    json!([
        {
            "name": "share_post",
            "description": format!(
                "Tell the other agents in this shared session something they would act on. \
                 kind \"working\": what you are about to change (which files, and why) when it could overlap with someone else's work. \
                 kind \"finding\": something you learned that would save another agent time. \
                 kind \"opinion\": ask everyone for a second opinion when you are unsure; any agent may answer, and the answers reach you like other news. \
                 One or two sentences in your own words, under {MAX_POST} characters. \
                 Never paste your person's prompt, code or secrets: everyone in the session reads posts."
            ),
            "inputSchema": {
                "type": "object",
                "properties": {
                    "kind": { "type": "string", "enum": ["working", "finding", "opinion"] },
                    "text": { "type": "string", "maxLength": MAX_POST },
                },
                "required": ["kind", "text"],
            },
        },
        {
            "name": "share_ask",
            "description": format!(
                "Ask one person's agent in this shared session about something they are working on or have looked into. \
                 Returns at once with the question's id; keep working. The answer reaches you with a tool result or your next prompt. \
                 If their person is away, a read-only copy of their agent answers. Under {MAX_POST} characters; never paste secrets."
            ),
            "inputSchema": {
                "type": "object",
                "properties": {
                    "to": { "type": "string", "description": "The person's GitHub login, as the session news names them." },
                    "text": { "type": "string", "maxLength": MAX_POST },
                },
                "required": ["to", "text"],
            },
        },
        {
            "name": "share_answer",
            "description": format!(
                "Answer a question another agent asked your person, or a second opinion you can help with. \
                 question: the id the news gave (like q7k2m9x4b). Answer from what you know, briefly, under {MAX_ANSWER} characters. \
                 If you don't know, say so. Never paste secrets."
            ),
            "inputSchema": {
                "type": "object",
                "properties": {
                    "question": { "type": "string", "description": "The question's id." },
                    "text": { "type": "string", "maxLength": MAX_ANSWER },
                },
                "required": ["question", "text"],
            },
        },
        {
            "name": "share_read",
            "description": "What the other agents and people in this shared session posted, pushed or changed since you last looked. \
                            News also arrives on its own with your tool results; call this to check before you decide something that others may affect.",
            "inputSchema": { "type": "object", "properties": {} },
        },
    ])
}

/// Run one tool. Err is a message for the agent, shown as a failed call.
fn call(name: &str, args: &Value) -> Result<String, String> {
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let at = Checkout::discover(&cwd).ok_or_else(|| Refused::NotShared.to_string())?;
    let b = session::binding(&at).ok_or_else(|| Refused::NotShared.to_string())?;
    let agent = std::env::var("CLAUDE_CODE_SESSION_ID").ok().filter(|s| !s.is_empty()).map(|s| format!("claude:{s}"));
    let now = now_ms();
    match name {
        "share_post" => {
            let text = args.get("text").and_then(Value::as_str).unwrap_or("");
            let kind = match args.get("kind").and_then(Value::as_str) {
                Some("working") => PostKind::Working,
                Some("finding") => PostKind::Finding,
                Some("opinion") => {
                    let id = talk::ask(&at, &b, agent.as_deref(), None, text, now).map_err(|e| e.to_string())?;
                    return Ok(format!("Asked everyone (question {id}). Answers reach you with a tool result or your next prompt; keep working."));
                }
                other => return Err(format!("kind must be \"working\", \"finding\" or \"opinion\", not {}", other.unwrap_or("missing"))),
            };
            talk::post(&at, &b, agent.as_deref(), kind, text, now).map_err(|e| e.to_string())?;
            Ok("Posted. The others' agents get it with their next tool result or prompt.".into())
        }
        "share_ask" => {
            let to = args.get("to").and_then(Value::as_str).unwrap_or("");
            let text = args.get("text").and_then(Value::as_str).unwrap_or("");
            if to.trim().is_empty() {
                return Err("Say whose agent to ask: to is their GitHub login.".into());
            }
            let id = talk::ask(&at, &b, agent.as_deref(), Some(to), text, now).map_err(|e| e.to_string())?;
            Ok(format!("Asked {to} (question {id}). The answer reaches you with a tool result or your next prompt; keep working."))
        }
        "share_answer" => {
            let question = args.get("question").and_then(Value::as_str).unwrap_or("");
            let text = args.get("text").and_then(Value::as_str).unwrap_or("");
            let background = std::env::var_os(crate::listen::BACKGROUND).is_some();
            let asker = talk::answer(&at, &b, agent.as_deref(), question, text, background, now).map_err(|e| e.to_string())?;
            Ok(format!("Answered. {asker}'s agent gets it with its next tool result or prompt."))
        }
        "share_read" => {
            let me = agent.unwrap_or_else(|| "claude:unknown".into());
            match talk::read(&at, &b, &me, now).map_err(|e| e.to_string())? {
                Some(t) => Ok(t),
                None => Ok("Nothing new in the shared session since you last looked.".into()),
            }
        }
        other => Err(format!("unknown tool {other}")),
    }
}
