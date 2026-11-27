//! Joining through the real binary. "GitHub" is a local bare repo that git
//! reaches through a global url.insteadOf, and `gh` is a fake on PATH that
//! answers from environment variables, so every join failure can be staged.

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;

const URL: &str = "https://github.com/deleteme/joindeleterepo.git";

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git").arg("-C").arg(dir).args(args)
        .env("GIT_AUTHOR_NAME", "t").env("GIT_AUTHOR_EMAIL", "t@t").env("GIT_COMMITTER_NAME", "t").env("GIT_COMMITTER_EMAIL", "t@t")
        .output().unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).into_owned()
}

const FAKE_GH: &str = r#"#!/bin/sh
case "$*" in
  "config get git_protocol"*) echo https ;;
  "api user"*) [ -n "$FAKE_GH_LOGIN" ] || { echo "To get started with GitHub CLI, please run: gh auth login" >&2; exit 4; }; echo "$FAKE_GH_LOGIN" ;;
  "api repos/"*)
    case "$FAKE_GH_ACCESS" in
      write) printf 'true\ttrue\n' ;;
      read) printf 'false\ttrue\n' ;;
      *) echo "gh: Not Found (HTTP 404)" >&2; exit 1 ;;
    esac ;;
  *) echo "fake gh: $*" >&2; exit 1 ;;
esac
"#;

struct World {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    remote: PathBuf,
    bin: PathBuf,
    gitconfig: PathBuf,
}

impl World {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_path_buf();
        let remote = root.join("remote.git");
        std::fs::create_dir(&remote).unwrap();
        git(&remote, &["init", "-q", "--bare", "-b", "google-search"]);
        let bin = root.join("bin");
        std::fs::create_dir(&bin).unwrap();
        std::fs::write(bin.join("gh"), FAKE_GH).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(bin.join("gh"), std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let gitconfig = root.join("gitconfig");
        std::fs::write(&gitconfig, format!("[url \"file://{}\"]\n\tinsteadOf = {URL}\n[protocol \"file\"]\n\tallow = always\n[uploadpack]\n\tallowFilter = true\n", remote.display())).unwrap();
        World { _tmp: tmp, root, remote, bin, gitconfig }
    }

    fn person(&self, login: &str) -> Person<'_> {
        let home = self.root.join(format!("{login}-home"));
        std::fs::create_dir_all(&home).unwrap();
        Person { login: login.into(), home, world: self }
    }
}

struct Person<'w> {
    login: String,
    home: PathBuf,
    world: &'w World,
}

impl Person<'_> {
    fn cli(&self, dir: &Path, args: &[&str], gh: &[(&str, &str)]) -> Value {
        let path = format!("{}:{}", self.world.bin.display(), std::env::var("PATH").unwrap());
        let mut c = Command::new(env!("CARGO_BIN_EXE_herdr-session"));
        c.args(args).current_dir(dir)
            .env("PATH", path)
            .env("HOME", &self.home)
            .env("HERDR_SESSION_HOME", self.home.join(".herdr/session"))
            .env("CLAUDE_CONFIG_DIR", self.home.join(".claude"))
            .env("GIT_CONFIG_GLOBAL", &self.world.gitconfig)
            .env("FAKE_GH_LOGIN", &self.login)
            .env("FAKE_GH_ACCESS", "write")
            .env_remove("HERDR_SESSION_LOGIN");
        for (k, v) in gh {
            c.env(k, v);
        }
        let out = c.output().unwrap();
        let v: Value = serde_json::from_slice(&out.stdout).unwrap_or_else(|_| panic!("one JSON envelope: {}", String::from_utf8_lossy(&out.stdout)));
        v
    }
    fn ok(&self, dir: &Path, args: &[&str]) -> Value {
        let v = self.cli(dir, args, &[]);
        assert_eq!(v["ok"], true, "{args:?}: {v}");
        v["value"].clone()
    }
    fn fails(&self, args: &[&str], gh: &[(&str, &str)]) -> (String, String) {
        let v = self.cli(&self.home, args, gh);
        assert_eq!(v["ok"], false, "{args:?} should fail: {v}");
        (v["error"]["kind"].as_str().unwrap().into(), v["error"]["message"].as_str().unwrap().into())
    }
    fn join(&self, link: &str, step: &str) -> Value {
        self.ok(&self.home, &["join", "--link", link, "--step", step])
    }
}

/// Thomas shares google-search; returns the link.
fn share(w: &World) -> (Person<'_>, PathBuf, String) {
    let thomas = w.person("thomas");
    let dir = w.root.join("thomas");
    std::fs::create_dir(&dir).unwrap();
    git(&dir, &["init", "-q", "-b", "google-search"]);
    git(&dir, &["remote", "add", "origin", URL]);
    std::fs::write(dir.join("query.py"), "def build():\n    return 1\n").unwrap();
    git(&dir, &["add", "."]);
    git(&dir, &["commit", "-q", "-m", "init"]);
    let push = Command::new("git").arg("-C").arg(&dir).args(["push", "-q", "-u", "origin", "google-search"]).env("GIT_CONFIG_GLOBAL", &w.gitconfig).output().unwrap();
    assert!(push.status.success());
    let s = thomas.ok(&dir, &["start", "--name", "Google Search"]);
    let link = s["link"].as_str().unwrap().to_string();
    (thomas, dir, link)
}

#[test]
fn ryan_joins_while_thomas_is_away() {
    let w = World::new();
    let (_thomas, _tdir, link) = share(&w);
    let ryan = w.person("ryan");

    // Each step passes and carries the header facts.
    let s = ryan.join(&link, "session");
    assert_eq!((s["owner"].as_str(), s["name"].as_str(), s["branch"].as_str()), (Some("thomas"), Some("Google Search"), Some("google-search")));
    assert_eq!(s["repo"], "deleteme/joindeleterepo");
    assert_eq!(ryan.join(&link, "github")["login"], "ryan");
    assert_eq!(ryan.join(&link, "access")["permission"], "write");
    let wt = PathBuf::from(ryan.join(&link, "folder")["worktree"].as_str().unwrap());
    assert_eq!(wt, ryan.home.join("herdr-sessions/joindeleterepo/google-search"));
    assert!(!wt.exists(), "the folder step only checks");
    ryan.join(&link, "branch");

    // A real checkout of the shared branch, tracking GitHub, bound to the session.
    assert_eq!(std::fs::read_to_string(wt.join("query.py")).unwrap(), "def build():\n    return 1\n");
    assert_eq!(git(&wt, &["rev-parse", "--abbrev-ref", "@{u}"]).trim(), "origin/google-search");
    let st = ryan.ok(&wt, &["status"]);
    assert_eq!((st["session"]["is_owner"].as_bool(), st["session"]["me"].as_str()), (Some(false), Some("ryan")));
    assert_eq!(st["session"]["code"], link.rsplit('/').next().unwrap());

    // Ryan's "joined" reached GitHub: Thomas sees him after a sync.
    assert!(st["session"]["members"].as_array().unwrap().iter().any(|m| m == "ryan"));
    let refs = git(&w.remote, &["for-each-ref", "--format=%(refname)", "refs/herdr/s/"]);
    assert_eq!(refs.lines().count(), 2, "Thomas's stream and Ryan's: {refs}");

    // Ryan's Claude hooks are in his own settings.
    let settings = std::fs::read_to_string(ryan.home.join(".claude/settings.json")).unwrap();
    assert!(settings.contains("herdr-session hook prompt"));

    // Joining again is quick and changes nothing: same worktree, no second "joined".
    ryan.join(&link, "branch");
    let refs_again = git(&w.remote, &["rev-list", "--count", "--all"]);
    ryan.join(&link, "branch");
    assert_eq!(git(&w.remote, &["rev-list", "--count", "--all"]), refs_again);
}

#[test]
fn each_step_says_why_it_failed() {
    let w = World::new();
    let (thomas, tdir, link) = share(&w);
    let ryan = w.person("ryan");
    let code = link.rsplit('/').next().unwrap();

    let bad = |args: &[&str], gh: &[(&str, &str)]| ryan.fails(args, gh).0;
    assert_eq!(bad(&["join", "--link", "herdr.dev/join/k7q3m9x4", "--step", "session"], &[]), "BadLink");
    let wrong = link.replace(code, "aaaaaaaa");
    assert_eq!(bad(&["join", "--link", &wrong, "--step", "session"], &[]), "SessionNotFound");
    assert_eq!(bad(&["join", "--link", &link, "--step", "github"], &[("FAKE_GH_LOGIN", "")]), "NotSignedIn");
    assert_eq!(bad(&["join", "--link", &link, "--step", "access"], &[("FAKE_GH_ACCESS", "none")]), "NoAccess");

    // Who can join is enforced by the client.
    thomas.ok(&tdir, &["change", "--who", "priya"]);
    assert_eq!(bad(&["join", "--link", &link, "--step", "access"], &[]), "NotInvited");
    thomas.ok(&tdir, &["change", "--who", "anyone"]);

    // A folder that is taken.
    let taken = ryan.home.join("herdr-sessions/joindeleterepo/google-search");
    std::fs::create_dir_all(&taken).unwrap();
    std::fs::write(taken.join("mine.txt"), "x").unwrap();
    let (kind, msg) = ryan.fails(&["join", "--link", &link, "--step", "folder"], &[]);
    assert_eq!(kind, "FolderUnusable");
    assert_eq!(msg, "Couldn't create ~/herdr-sessions/joindeleterepo/google-search. It may already exist, or herdr can't write there.");
    // "~/" means the home folder.
    let v = ryan.ok(&ryan.home, &["join", "--link", &link, "--step", "folder", "--folder", "~/work"]);
    assert_eq!(v["worktree"], ryan.home.join("work/joindeleterepo/google-search").to_str().unwrap());
    // Another folder works.
    let other = w.root.join("elsewhere");
    let v = ryan.ok(&ryan.home, &["join", "--link", &link, "--step", "folder", "--folder", other.to_str().unwrap()]);
    assert_eq!(v["worktree"], other.join("joindeleterepo/google-search").to_str().unwrap());

    // The branch is gone from GitHub.
    git(&w.remote, &["update-ref", "-d", "refs/heads/google-search"]);
    assert_eq!(bad(&["join", "--link", &link, "--step", "branch", "--folder", other.to_str().unwrap()], &[]), "BranchGone");

    // A closed session.
    thomas.ok(&tdir, &["change", "--sharing", "off"]);
    assert_eq!(bad(&["join", "--link", &link, "--step", "session"], &[]), "Closed");
}

#[test]
fn read_only_people_join_but_publish_nothing() {
    let w = World::new();
    let (_thomas, _tdir, link) = share(&w);
    let amy = w.person("amy");
    let v = amy.cli(&amy.home, &["join", "--link", &link, "--step", "branch"], &[("FAKE_GH_ACCESS", "read")]);
    assert_eq!(v["ok"], true, "{v}");
    assert_eq!(v["value"]["permission"], "read");
    let refs = git(&w.remote, &["for-each-ref", "--format=%(refname)", "refs/herdr/s/"]);
    assert_eq!(refs.lines().count(), 1, "only Thomas's stream: {refs}");
}
