//! herdr-session: one-shot CLI. argv -> one verb -> one JSON envelope on stdout.
//! Exit 0 for both ok and error envelopes; nonzero means the engine itself broke.
//! `hook <event>` is the exception: it prints only what Claude Code expects.

use herdr_session::claude;
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
        let Some(event) = args.get(1).and_then(|e| claude::HookEvent::parse(e)) else { return };
        let reply = claude::hook(event, &input);
        if let Some(out) = &reply.out {
            use std::io::Write;
            let mut stdout = std::io::stdout();
            if writeln!(stdout, "{out}").and_then(|_| stdout.flush()).is_err() {
                return; // Claude did not get it: keep the cursor where it was
            }
        }
        if let Some(after) = reply.after {
            after();
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
