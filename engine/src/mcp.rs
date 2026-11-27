//! `herdr-session mcp`: the tools Claude agents use to talk to each other
//! (SPEC 9.1). An MCP server over stdio, one JSON-RPC message per line.
//! Claude Code starts it in the project folder and passes the conversation's
//! id in CLAUDE_CODE_SESSION_ID (checked 2026-11-26, Claude Code 2.1.332).

use crate::git::Checkout;
use crate::ledger::PostKind;
use crate::session;
use crate::talk::{self, Refused, MAX_POST};
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
                 One or two sentences in your own words, under {MAX_POST} characters. \
                 Never paste your person's prompt, code or secrets: everyone in the session reads posts."
            ),
            "inputSchema": {
                "type": "object",
                "properties": {
                    "kind": { "type": "string", "enum": ["working", "finding"] },
                    "text": { "type": "string", "maxLength": MAX_POST },
                },
                "required": ["kind", "text"],
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
            let kind = match args.get("kind").and_then(Value::as_str) {
                Some("working") => PostKind::Working,
                Some("finding") => PostKind::Finding,
                other => return Err(format!("kind must be \"working\" or \"finding\", not {}", other.unwrap_or("missing"))),
            };
            let text = args.get("text").and_then(Value::as_str).unwrap_or("");
            talk::post(&at, &b, agent.as_deref(), kind, text, now).map_err(|e| e.to_string())?;
            Ok("Posted. The others' agents get it with their next tool result or prompt.".into())
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
