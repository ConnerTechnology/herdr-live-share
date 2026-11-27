//! herdr-session: one-shot CLI. argv -> one verb -> one JSON envelope on stdout:
//! {"api":1,"ok":true,"value":…} or {"api":1,"ok":false,"error":{"kind","message"}}.
//! Exit 0 for both; nonzero means the engine itself broke.
//! `hook <event>` is the exception: it prints only what Claude Code expects.

use herdr_session::api::{self, ApiError, Change};
use herdr_session::{claude, join, mcp};
use herdr_session::util::now_ms;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::PathBuf;

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

    if verb == "mcp" {
        // The agents' tools (SPEC 9.1): JSON-RPC over stdio until Claude closes it.
        mcp::serve(std::io::stdin().lock(), std::io::stdout().lock());
        return;
    }

    let opts = Opts::parse(&args[args.len().min(1)..]);
    let envelope = match run(verb, &opts) {
        Ok(value) => json!({ "api": API, "ok": true, "value": value }),
        Err(e) => json!({ "api": API, "ok": false, "error": e }),
    };
    println!("{envelope}");
}

fn run(verb: &str, o: &Opts) -> Result<Value, ApiError> {
    let now = now_ms();
    match verb {
        "version" => Ok(json!({ "version": env!("CARGO_PKG_VERSION"), "api": API })),
        "status" => to_json(api::status(&o.checkout()?, now)?),
        "start" => to_json(api::start(&o.checkout()?, o.get("name").unwrap_or(""), now)?),
        "change" => {
            let at = o.checkout()?;
            if let Some(v) = o.get("keep-in-sync") {
                return to_json(api::set_keep_in_sync(&at, on(v)?, now)?);
            }
            let c = if let Some(v) = o.get("sharing") {
                Change::Sharing(on(v)?)
            } else if let Some(v) = o.get("name") {
                Change::Name(v.to_string())
            } else if let Some(v) = o.get("auto-close") {
                Change::AutoClose(on(v)?)
            } else if let Some(v) = o.get("who") {
                let logins: BTreeSet<String> = if v == "anyone" { BTreeSet::new() } else { v.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect() };
                Change::WhoCanJoin(logins)
            } else {
                return Err(usage("change needs one of --sharing, --name, --auto-close, --keep-in-sync, --who"));
            };
            to_json(api::change(&at, c, now)?)
        }
        "collaborators" => to_json(api::collaborators(&o.checkout()?)?),
        "join" => {
            let link = o.get("link").ok_or_else(|| usage("join needs --link"))?;
            let step = o.get("step").and_then(join::Step::parse).ok_or_else(|| usage("join needs --step session|github|access|folder|branch"))?;
            to_json(join::step(link, step, o.get("folder").map(std::path::Path::new), now)?)
        }
        "hooks" => {
            let io = |e: std::io::Error| ApiError { kind: "Io", message: e.to_string() };
            let changed = if o.0.contains_key("uninstall") { claude::uninstall_hooks().map_err(io)? } else { claude::install_hooks().map_err(io)? };
            Ok(json!({ "changed": changed, "settings": claude::settings_path() }))
        }
        "drain" => {
            let at = o.checkout()?;
            let b = herdr_session::session::binding(&at).ok_or_else(|| ApiError { kind: "NotShared", message: "This space is not shared.".into() })?;
            let ledger_only = o.0.contains_key("ledger");
            to_json(herdr_session::drain::drain(&at, &b, ledger_only).map_err(ApiError::from)?)
        }
        "sync" | "timeline" | "say" | "doctor" => {
            Err(ApiError { kind: "NotImplemented", message: format!("`{verb}` is not built yet") })
        }
        "" => Err(usage("usage: herdr-session <verb> [--cwd DIR] [options]")),
        other => Err(usage(&format!("unknown verb `{other}`"))),
    }
}

fn to_json(v: impl Serialize) -> Result<Value, ApiError> {
    Ok(serde_json::to_value(v).expect("replies serialize"))
}

fn usage(m: &str) -> ApiError {
    ApiError { kind: "Usage", message: m.to_string() }
}

fn on(v: &str) -> Result<bool, ApiError> {
    match v {
        "on" | "true" => Ok(true),
        "off" | "false" => Ok(false),
        _ => Err(usage(&format!("expected on or off, got `{v}`"))),
    }
}

/// `--key value` pairs, bare `--flag`s and bare words.
struct Opts(BTreeMap<String, Option<String>>);

impl Opts {
    fn parse(args: &[String]) -> Self {
        let mut m = BTreeMap::new();
        let mut i = 0;
        while i < args.len() {
            if let Some(k) = args[i].strip_prefix("--") {
                let v = args.get(i + 1).filter(|v| !v.starts_with("--")).cloned();
                i += if v.is_some() { 2 } else { 1 };
                m.insert(k.to_string(), v);
            } else {
                // A bare word (`hooks uninstall`) reads as a flag.
                m.insert(args[i].clone(), None);
                i += 1;
            }
        }
        Opts(m)
    }
    fn get(&self, k: &str) -> Option<&str> {
        self.0.get(k).and_then(|v| v.as_deref())
    }
    /// --cwd, else the process's directory (mods set it through exec's cwd).
    fn checkout(&self) -> Result<herdr_session::git::Checkout, ApiError> {
        let cwd = self.get("cwd").map(PathBuf::from).unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
        api::checkout(&cwd)
    }
}
