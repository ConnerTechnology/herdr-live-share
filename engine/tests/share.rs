//! Sharing a space through the real binary: start, change, status. "GitHub" is a local bare repo: the clone's origin is a github.com
//! URL that git rewrites (url.<file>.insteadOf) to the bare repo, so the engine
//! sees exactly what it would see for a real GitHub remote.

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;

const URL: &str = "https://github.com/deleteme/joindeleterepo.git";

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@t")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@t")
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).into_owned()
}

struct World {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    remote: PathBuf,
}

impl World {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_path_buf();
        let remote = root.join("remote.git");
        std::fs::create_dir(&remote).unwrap();
        git(&remote, &["init", "-q", "--bare", "-b", "google-search"]);
        World { _tmp: tmp, root, remote }
    }

    /// A clone of "GitHub" for `login`, with its own engine home.
    fn clone(&self, login: &str) -> Person {
        let dir = self.root.join(login);
        std::fs::create_dir(&dir).unwrap();
        git(&dir, &["init", "-q", "-b", "google-search"]);
        git(&dir, &["config", &format!("url.file://{}.insteadOf", self.remote.display()), URL]);
        git(&dir, &["remote", "add", "origin", URL]);
        if !git(&self.remote, &["for-each-ref", "refs/heads"]).is_empty() {
            git(&dir, &["fetch", "-q", "origin"]);
            git(&dir, &["checkout", "-q", "-B", "google-search", "--track", "origin/google-search"]);
        }
        Person { login: login.into(), dir, home: self.root.join(format!("{login}-home")) }
    }
}

struct Person {
    login: String,
    dir: PathBuf,
    home: PathBuf,
}

impl Person {
    fn cli(&self, args: &[&str]) -> Value {
        let out = Command::new(env!("CARGO_BIN_EXE_herdr-session"))
            .args(args)
            .current_dir(&self.dir)
            .env("HERDR_SESSION_HOME", &self.home)
            .env("HERDR_SESSION_LOGIN", &self.login)
            // Sharing installs Claude hooks: into this person's own folder, never the real ~/.claude.
            .env("HOME", &self.home)
            .env("CLAUDE_CONFIG_DIR", self.home.join(".claude"))
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .output()
            .unwrap();
        assert!(out.status.success(), "the engine exits 0 even on errors");
        let v: Value = serde_json::from_slice(&out.stdout).expect("one JSON envelope");
        assert_eq!(v["api"], 1);
        v
    }
    fn ok(&self, args: &[&str]) -> Value {
        let v = self.cli(args);
        assert_eq!(v["ok"], true, "{args:?}: {v}");
        v["value"].clone()
    }
    fn err(&self, args: &[&str]) -> String {
        let v = self.cli(args);
        assert_eq!(v["ok"], false, "{args:?} should fail: {v}");
        v["error"]["kind"].as_str().unwrap().to_string()
    }
    fn write(&self, file: &str, text: &str) {
        std::fs::write(self.dir.join(file), text).unwrap();
    }
    fn commit(&self, file: &str, text: &str, msg: &str) {
        self.write(file, text);
        git(&self.dir, &["add", "."]);
        git(&self.dir, &["commit", "-q", "-m", msg]);
    }
}

#[test]
fn share_a_space_end_to_end() {
    let w = World::new();
    let thomas = w.clone("thomas");
    thomas.commit("query.py", "def build():\n    return 1\n", "init");
    git(&thomas.dir, &["push", "-q", "-u", "origin", "google-search"]);

    // Not shared yet.
    let s = thomas.ok(&["status"]);
    assert_eq!(s["session"], Value::Null);
    assert_eq!(s["branch"], "google-search");
    assert_eq!(s["remote"], serde_json::json!({ "kind": "github", "owner": "deleteme", "name": "joindeleterepo" }));

    // Uncommitted and unpushed work does not stop sharing.
    thomas.commit("ranking.py", "boost = 0.2\n", "Add ranking");
    thomas.write("notes.md", "todo\n");

    // Start: a new session, link only after the genesis is on "GitHub".
    let s = thomas.ok(&["start", "--name", "Google Search"]);
    let code = s["code"].as_str().unwrap().to_string();
    assert_eq!(code.len(), 8);
    assert_eq!(s["link"], format!("https://herdr.dev/join/deleteme/joindeleterepo/{code}"));
    assert_eq!((s["open"].as_bool(), s["is_owner"].as_bool(), s["name"].as_str()), (Some(true), Some(true), Some("Google Search")));
    assert_eq!((s["auto_close"].as_bool(), s["keep_in_sync"].as_bool()), (Some(false), Some(true)), "defaults: auto-close off, keep in sync on");
    assert_eq!(s["who_can_join"], serde_json::json!({ "anyone": true, "logins": [] }));
    assert!(git(&w.remote, &["for-each-ref", &format!("refs/herdr/s/{code}/")]).lines().count() == 1);

    // Sharing installed the Claude hooks, once, keeping nothing else out.
    let settings: Value = serde_json::from_slice(&std::fs::read(thomas.home.join(".claude/settings.json")).unwrap()).unwrap();
    let prompt = settings["hooks"]["UserPromptSubmit"][0]["hooks"][0]["command"].as_str().unwrap();
    assert!(prompt.ends_with("herdr-session hook prompt"), "{prompt}");
    assert_eq!(settings["hooks"]["PreToolUse"][0]["matcher"], "Edit|Write|MultiEdit|NotebookEdit");

    // Start twice: the same session.
    assert_eq!(thomas.ok(&["start", "--name", "Google Search"])["code"], code.as_str());

    // Settings.
    assert_eq!(thomas.ok(&["change", "--name", "Search v2"])["name"], "Search v2");
    assert_eq!(thomas.ok(&["change", "--auto-close", "on"])["auto_close"], true);
    let s = thomas.ok(&["change", "--auto-close", "on"]);
    assert_eq!(s["synced"], true, "no change, nothing to push");
    assert_eq!(thomas.ok(&["change", "--keep-in-sync", "off"])["keep_in_sync"], false);
    assert_eq!(thomas.ok(&["change", "--who", "ryan,priya"])["who_can_join"], serde_json::json!({ "anyone": false, "logins": ["priya", "ryan"] }));
    assert_eq!(thomas.ok(&["change", "--who", "anyone"])["who_can_join"]["anyone"], true);

    // Off closes; on reopens the same session with the same link.
    let s = thomas.ok(&["change", "--sharing", "off"]);
    assert_eq!((s["open"].as_bool(), s["closed_by"].as_str()), (Some(false), Some("owner")));
    assert_eq!(s["keep_in_sync"], false, "Thomas's own setting survives closing");
    let s = thomas.ok(&["start", "--name", "Search v2"]);
    assert_eq!((s["open"].as_bool(), s["code"].as_str()), (Some(true), Some(code.as_str())));

    // Status is local and shows the same thing.
    let st = thomas.ok(&["status"]);
    assert_eq!((st["session"]["code"].as_str(), st["session"]["name"].as_str()), (Some(code.as_str()), Some("Search v2")));

    // Every change reached "GitHub": a fresh clone of the ledger folds to the same settings.
    let events = git(&w.remote, &["rev-list", "--count", &format!("refs/herdr/s/{code}/m/{}", s_tag(&w, &code))]);
    assert_eq!(events.trim(), "7", "opened, name, auto-close, who, who, closed, reopened (no-op changes and keep in sync add nothing)");

    // Someone else's checkout bound to the session cannot change it.
    let ryan = w.clone("ryan");
    let ryan_bind = serde_json::json!({ "sid": sid_of(&thomas), "remote": "origin", "login": "ryan" });
    let bindings = PathBuf::from(git(&ryan.dir, &["rev-parse", "--path-format=absolute", "--git-common-dir"]).trim()).join("herdr-session/bindings");
    std::fs::create_dir_all(&bindings).unwrap();
    std::fs::write(bindings.join("google-search.json"), ryan_bind.to_string()).unwrap();
    git(&ryan.dir, &["fetch", "-q", "origin", &format!("+refs/herdr/s/{code}/*:refs/herdr-seen/{code}/*")]);
    assert_eq!(ryan.err(&["change", "--name", "Ryan's"]), "NotOwner");
    // Keep in sync is per person: Ryan has his own, on by default, and can change it.
    assert_eq!(ryan.ok(&["status"])["session"]["keep_in_sync"], true, "Thomas turned his off; Ryan's is still on");
    assert_eq!(ryan.ok(&["change", "--keep-in-sync", "off"])["keep_in_sync"], false);
    assert_eq!(thomas.ok(&["status"])["session"]["keep_in_sync"], false);
    let events_after = git(&w.remote, &["rev-list", "--count", &format!("refs/herdr/s/{code}/m/{}", s_tag(&w, &code))]);
    assert_eq!(events_after.trim(), "7", "keep in sync writes no session event");
    assert_eq!(ryan.ok(&["status"])["session"]["is_owner"], false);
}

/// The tag of the one stream on the remote.
fn s_tag(w: &World, code: &str) -> String {
    let refs = git(&w.remote, &["for-each-ref", "--format=%(refname)", &format!("refs/herdr/s/{code}/")]);
    refs.trim().rsplit('/').next().unwrap().to_string()
}

fn sid_of(p: &Person) -> String {
    let common = PathBuf::from(git(&p.dir, &["rev-parse", "--path-format=absolute", "--git-common-dir"]).trim());
    let b: Value = serde_json::from_slice(&std::fs::read(common.join("herdr-session/bindings/google-search.json")).unwrap()).unwrap();
    b["sid"].as_str().unwrap().to_string()
}

#[test]
fn refusals() {
    let w = World::new();
    let thomas = w.clone("thomas");
    thomas.commit("a.txt", "a\n", "init");
    git(&thomas.dir, &["push", "-q", "-u", "origin", "google-search"]);

    // A branch never pushed.
    git(&thomas.dir, &["checkout", "-q", "-b", "local-only"]);
    assert_eq!(thomas.err(&["start", "--name", "x"]), "NotPushed");

    // A remote that is not GitHub.
    git(&thomas.dir, &["checkout", "-q", "google-search"]);
    git(&thomas.dir, &["remote", "set-url", "origin", &format!("file://{}", w.remote.display())]);
    assert_eq!(thomas.ok(&["status"])["remote"]["kind"], "other");
    assert_eq!(thomas.err(&["start", "--name", "x"]), "NotGitHub");

    // Not a repo; not shared; a bad option.
    let plain = Person { login: "thomas".into(), dir: w.root.clone(), home: thomas.home.clone() };
    assert_eq!(plain.err(&["status"]), "NotARepo");
    assert_eq!(thomas.err(&["change", "--name", "x"]), "NotShared");
    assert_eq!(thomas.err(&["change", "--auto-close", "maybe"]), "Usage");
}
