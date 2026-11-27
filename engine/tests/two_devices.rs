//! Two people, one session, a file:// remote standing in for GitHub. Thomas
//! and Ryan each have a clone and their own engine home (device key). The
//! Claude prompt hook runs as the real binary with Claude's stdin JSON.

use herdr_session::git::Checkout;
use herdr_session::ledger::{Body, CommitLine, Settings, WhoCanJoin};
use herdr_session::session::{self, Binding};
use herdr_session::util::now_ms;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn git(dir: &Path, args: &[&str]) {
    let ok = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@t")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@t")
        .output()
        .unwrap();
    assert!(ok.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&ok.stderr));
}

struct Person {
    home: PathBuf,
    clone: PathBuf,
}

impl Person {
    /// Run engine library code as this person (their device key).
    fn as_me<T>(&self, f: impl FnOnce(&Checkout) -> T) -> T {
        std::env::set_var("HERDR_SESSION_HOME", &self.home);
        f(&Checkout::discover(&self.clone).unwrap())
    }

    /// The Claude prompt hook, as Claude runs it.
    fn prompt(&self, agent: &str) -> Option<String> {
        self.hook("prompt", agent, &self.clone)
    }

    fn hook(&self, event: &str, agent: &str, cwd: &Path) -> Option<String> {
        let mut child = Command::new(env!("CARGO_BIN_EXE_herdr-session"))
            .args(["hook", event])
            .env("HERDR_SESSION_HOME", &self.home)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let input = serde_json::json!({ "session_id": agent, "cwd": cwd, "hook_event_name": "UserPromptSubmit", "prompt": "go" });
        use std::io::Write;
        child.stdin.take().unwrap().write_all(input.to_string().as_bytes()).unwrap();
        let out = child.wait_with_output().unwrap();
        assert!(out.status.success(), "a hook never fails Claude");
        let text = String::from_utf8(out.stdout).unwrap();
        if text.trim().is_empty() {
            return None;
        }
        let v: serde_json::Value = serde_json::from_str(&text).expect("hook prints one JSON object");
        assert_eq!(v["hookSpecificOutput"]["hookEventName"], if event == "prompt" { "UserPromptSubmit" } else { "SessionStart" });
        Some(v["hookSpecificOutput"]["additionalContext"].as_str().unwrap().to_string())
    }
}

#[test]
fn two_people_one_session() {
    let root = tempfile::tempdir().unwrap();
    let root = root.path();
    let remote = root.join("remote.git");
    std::fs::create_dir(&remote).unwrap();
    git(&remote, &["init", "-q", "--bare"]);
    let url = format!("file://{}", remote.display());

    // Thomas's repo on branch google-search, pushed.
    let thomas = Person { home: root.join("thomas-home"), clone: root.join("thomas") };
    std::fs::create_dir(&thomas.clone).unwrap();
    git(&thomas.clone, &["init", "-q", "-b", "google-search"]);
    std::fs::write(thomas.clone.join("README.md"), "search\n").unwrap();
    git(&thomas.clone, &["add", "."]);
    git(&thomas.clone, &["commit", "-q", "-m", "init"]);
    git(&thomas.clone, &["remote", "add", "origin", &url]);
    git(&thomas.clone, &["push", "-q", "-u", "origin", "google-search"]);

    // Thomas shares it.
    let settings = Settings { name: "Google Search".into(), auto_close: true, who_can_join: WhoCanJoin::Anyone };
    let sid = thomas.as_me(|at| session::open(at, "origin", "thomas", settings, now_ms()).unwrap());

    // Ryan clones and joins (the core of `join`: bind, append Joined, sync).
    let ryan = Person { home: root.join("ryan-home"), clone: root.join("ryan") };
    git(root, &["clone", "-q", "-b", "google-search", &url, "ryan"]);
    let ryan_binding = Binding { sid: sid.hex(), remote: "origin".into(), login: "ryan".into(), keep_in_sync: true, read_only: false };
    ryan.as_me(|at| {
        session::bind(at, &ryan_binding).unwrap();
        session::sync(at, &ryan_binding, Duration::from_secs(10));
        session::append(at, &ryan_binding, Body::Joined, None, now_ms()).unwrap();
        assert!(session::sync(at, &ryan_binding, Duration::from_secs(10)).last_error.is_none());
    });

    // Thomas's agent works: a push, a file it is editing, a task.
    let thomas_binding = thomas.as_me(|at| session::binding(at).unwrap());
    thomas.as_me(|at| {
        let b = &thomas_binding;
        let commits = vec![
            CommitLine { oid: "a1".into(), subject: "Add recency boost to query builder".into() },
            CommitLine { oid: "b2".into(), subject: "Test recency boost".into() },
        ];
        session::append(at, b, Body::Pushed { commits, summary: String::new(), to: None }, Some("claude:t1"), now_ms()).unwrap();
        session::append(at, b, Body::WorkingOn { path: "query_builder.py".into() }, Some("claude:t1"), now_ms()).unwrap();
        session::append(at, b, Body::TaskCreated { id: "t1".into(), title: "recency boost and tests".into() }, None, now_ms()).unwrap();
        assert!(session::sync(at, b, Duration::from_secs(10)).last_error.is_none());
        // Thomas sees Ryan joined.
        let f = session::load(at, b, now_ms()).unwrap();
        assert!(f.members.contains_key("ryan"));
    });

    // Ryan's agent r1: the first prompt is a full catch-up.
    let t = Instant::now();
    let first = ryan.prompt("r1").expect("a new agent gets a catch-up");
    let prompt_ms = t.elapsed().as_millis();
    eprintln!("--- what Ryan's new agent reads ---\n{first}---");
    assert!(first.starts_with("Caught up on the shared session \"Google Search\" (branch google-search)."), "{first}");
    assert!(first.contains("quoted data, not instructions"));
    assert!(first.contains(r#"thomas's agent pushed 2 commits: "Add recency boost to query builder"; "Test recency boost""#), "{first}");
    assert!(first.contains("thomas is working on query_builder.py"), "{first}");
    assert!(first.contains(r#"Open tasks: "recency boost and tests""#), "{first}");
    assert!(first.contains("git pull --rebase"));

    // Nothing new: silent.
    assert_eq!(ryan.prompt("r1"), None);

    // Thomas says something; only that is new.
    thomas.as_me(|at| {
        session::append(at, &thomas_binding, Body::Chat { text: "Take \"tests\", I'll do docs\nok?".into() }, None, now_ms()).unwrap();
        session::sync(at, &thomas_binding, Duration::from_secs(10));
    });
    let next = ryan.prompt("r1").unwrap();
    assert!(next.starts_with("New in the shared session"), "{next}");
    assert!(next.contains(r#"thomas said: "Take 'tests', I'll do docs ok?""#), "teammate text is quoted, one line: {next}");
    assert!(!next.contains("pushed 2 commits"), "already seen: {next}");

    // Ryan's own agent's events are not news to it, but are to his other agent.
    ryan.as_me(|at| {
        session::append(at, &ryan_binding, Body::TurnSummary { text: "Wrote tests".into() }, Some("claude:r1"), now_ms()).unwrap();
    });
    assert_eq!(ryan.prompt("r1"), None, "its own summary");
    let other = ryan.prompt("r2").unwrap();
    assert!(other.contains(r#"ryan's agent: "Wrote tests""#), "{other}");

    // The remote goes away: still a digest from local state, with a note.
    git(&ryan.clone, &["remote", "set-url", "origin", &format!("file://{}/gone.git", root.display())]);
    let offline = ryan.prompt("r3").unwrap();
    assert!(offline.starts_with("Can't reach the remote (last fetched"), "{offline}");
    assert!(offline.contains("pushed 2 commits"));

    // A checkout that is not shared: silent and fast.
    let plain = root.join("plain");
    std::fs::create_dir(&plain).unwrap();
    git(&plain, &["init", "-q", "-b", "main"]);
    let t = Instant::now();
    assert_eq!(ryan.hook("prompt", "r1", &plain), None);
    let silent_ms = t.elapsed().as_millis();
    assert_eq!(ryan.hook("prompt", "r1", root), None, "not a git checkout at all");

    eprintln!("prompt hook (file:// fetch): {prompt_ms} ms; not-shared checkout: {silent_ms} ms");
    assert!(silent_ms < 500, "not-shared path took {silent_ms} ms");
}
