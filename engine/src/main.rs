//! herdr-session: one-shot CLI. argv -> one verb -> one JSON envelope on stdout.
//! Exit 0 for both ok and error envelopes; nonzero means the engine itself broke.
//! `hook <event>` is the exception: it prints only what Claude Code expects.

mod api;
mod claude;
mod drain;
mod git;
mod github;
mod ledger;
mod store;
mod view;

use serde_json::{json, Value};
use std::io::Read;

pub const API: u32 = 1;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let verb = args.first().map(String::as_str).unwrap_or("");

    if verb == "hook" {
        // Infallible by design: whatever happens, Claude gets valid output and exit 0.
        let mut input = String::new();
        let _ = std::io::stdin().read_to_string(&mut input);
        let event = args.get(1).and_then(|e| claude::HookEvent::parse(e));
        if let Some(out) = event.and_then(|e| claude::hook(e, &input)) {
            println!("{out}");
        }
        return;
    }

    let envelope = match verb {
        "version" => ok(json!({ "version": env!("CARGO_PKG_VERSION"), "api": API })),
        "preflight" | "publish" | "start" | "change" | "status" | "collaborators" | "join" | "sync"
        | "timeline" | "say" | "hooks" | "doctor" | "drain" => err("NotImplemented", &format!("`{verb}` is not built yet")),
        "" => err("Usage", "usage: herdr-session <verb> [args]"),
        other => err("Usage", &format!("unknown verb `{other}`")),
    };
    println!("{envelope}");
}

fn ok(value: Value) -> Value {
    json!({ "api": API, "ok": true, "value": value })
}

fn err(kind: &str, message: &str) -> Value {
    json!({ "api": API, "ok": false, "error": { "kind": kind, "message": message } })
}
