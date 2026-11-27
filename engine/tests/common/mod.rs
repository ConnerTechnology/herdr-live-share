//! Shared by the tests that run the real binary as several people: "GitHub" is a
//! local bare repo reached through a global url.insteadOf, and `gh` is a fake on
//! PATH that answers from environment variables.
#![allow(dead_code)]

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;

pub const URL: &str = "https://github.com/deleteme/joindeleterepo.git";

pub fn git(dir: &Path, args: &[&str]) -> String {
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

/// `claude mcp add|remove --scope user herdr-live-share …`, recorded in this
/// person's own config; and `claude -p …` (a background answer), which records
/// its arguments, folder and environment in claude-p.log, then answers the
/// question its prompt names through the real tools, as the fork would.
/// $FAKE_CLAUDE_SILENT: it runs but never answers.
const FAKE_CLAUDE: &str = r#"#!/bin/sh
dir="${CLAUDE_CONFIG_DIR:-$HOME}"
mkdir -p "$dir"
if [ "$1" = "-p" ]; then
  for a; do last="$a"; done
  { printf 'args:'; for a; do [ "$a" = "$last" ] || printf ' %s' "$a"; done; echo
    echo "cwd: $(pwd)"
    echo "background: ${HERDR_SESSION_BACKGROUND:-no}"
    echo "prompt: $last"; } >> "$dir/claude-p.log"
  [ -n "$FAKE_CLAUDE_SILENT" ] && exit 0
  id=$(printf '%s' "$last" | grep -o 'question "q[a-z0-9]*"' | head -n 1 | cut -d'"' -f2)
  printf '%s\n' '{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}' \
    "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{\"name\":\"share_answer\",\"arguments\":{\"question\":\"$id\",\"text\":\"Collaborators are cached for 10 minutes in github.rs.\"}}}" \
    | CLAUDE_CODE_SESSION_ID=fork-1 HERDR_SESSION_DRAIN_INLINE=1 "$HERDR_TEST_ENGINE" mcp >> "$dir/claude-p.log"
  exit 0
fi
echo "$*" >> "$dir/claude-calls.log"
case "$1 $2" in
  "mcp add") shift 6; printf '{"mcpServers":{"herdr-live-share":{"type":"stdio","command":"%s","args":["mcp"]}}}\n' "$1" > "$dir/.claude.json" ;;
  "mcp remove") printf '{"mcpServers":{}}\n' > "$dir/.claude.json" ;;
esac
"#;

pub struct World {
    _tmp: tempfile::TempDir,
    pub root: PathBuf,
    pub remote: PathBuf,
    pub bin: PathBuf,
    pub gitconfig: PathBuf,
}

impl World {
    pub fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_path_buf();
        let remote = root.join("remote.git");
        std::fs::create_dir(&remote).unwrap();
        git(&remote, &["init", "-q", "--bare", "-b", "google-search"]);
        let bin = root.join("bin");
        std::fs::create_dir(&bin).unwrap();
        std::fs::write(bin.join("gh"), FAKE_GH).unwrap();
        std::fs::write(bin.join("claude"), FAKE_CLAUDE).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for tool in ["gh", "claude"] {
                std::fs::set_permissions(bin.join(tool), std::fs::Permissions::from_mode(0o755)).unwrap();
            }
        }
        let gitconfig = root.join("gitconfig");
        std::fs::write(&gitconfig, format!("[url \"file://{}\"]\n\tinsteadOf = {URL}\n[protocol \"file\"]\n\tallow = always\n[uploadpack]\n\tallowFilter = true\n", remote.display())).unwrap();
        World { _tmp: tmp, root, remote, bin, gitconfig }
    }

    pub fn person(&self, login: &str) -> Person<'_> {
        let home = self.root.join(format!("{login}-home"));
        std::fs::create_dir_all(&home).unwrap();
        Person { login: login.into(), home, world: self }
    }
}

pub struct Person<'w> {
    pub login: String,
    pub home: PathBuf,
    pub world: &'w World,
}

impl Person<'_> {
    pub fn cli(&self, dir: &Path, args: &[&str], gh: &[(&str, &str)]) -> Value {
        let path = format!("{}:{}", self.world.bin.display(), std::env::var("PATH").unwrap());
        let mut c = Command::new(env!("CARGO_BIN_EXE_herdr-session"));
        c.args(args).current_dir(dir)
            .env("PATH", path)
            .env("HOME", &self.home)
            .env("HERDR_SESSION_HOME", self.home.join(".herdr/session"))
            .env("HERDR_SESSION_NO_LISTEN", "1")
            .env("CLAUDE_CONFIG_DIR", self.home.join(".claude"))
            .env("HERDR_SESSION_CLAUDE", self.world.bin.join("claude"))
            .env("HERDR_TEST_ENGINE", env!("CARGO_BIN_EXE_herdr-session"))
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
    /// Run a Claude hook as Claude would: input on stdin, its output back. Drains run inline.
    pub fn hook(&self, dir: &Path, event: &str, input: Value) -> String {
        use std::io::Write;
        let path = format!("{}:{}", self.world.bin.display(), std::env::var("PATH").unwrap());
        let mut child = Command::new(env!("CARGO_BIN_EXE_herdr-session"))
            .args(["hook", event])
            .current_dir(dir)
            .env("PATH", path)
            .env("HOME", &self.home)
            .env("HERDR_SESSION_HOME", self.home.join(".herdr/session"))
            .env("HERDR_SESSION_NO_LISTEN", "1")
            .env("CLAUDE_CONFIG_DIR", self.home.join(".claude"))
            .env("GIT_CONFIG_GLOBAL", &self.world.gitconfig)
            .env("FAKE_GH_LOGIN", &self.login)
            .env("FAKE_GH_ACCESS", "write")
            .env("HERDR_SESSION_DRAIN_INLINE", "1")
            .env_remove("HERDR_SESSION_LOGIN")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(input.to_string().as_bytes()).unwrap();
        let out = child.wait_with_output().unwrap();
        assert!(out.status.success(), "hooks never fail Claude");
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    /// Talk to `herdr-session mcp` as Claude does, from `dir` in conversation
    /// `session`: initialize, then each tool call. Returns each call's
    /// (text, is_error). Background pushes run inline.
    pub fn mcp(&self, dir: &Path, session: &str, calls: &[(&str, Value)]) -> Vec<(String, bool)> {
        use std::io::Write;
        let mut child = Command::new(env!("CARGO_BIN_EXE_herdr-session"))
            .arg("mcp")
            .current_dir(dir)
            .env("HOME", &self.home)
            .env("HERDR_SESSION_HOME", self.home.join(".herdr/session"))
            .env("HERDR_SESSION_NO_LISTEN", "1")
            .env("CLAUDE_CONFIG_DIR", self.home.join(".claude"))
            .env("CLAUDE_CODE_SESSION_ID", session)
            .env("GIT_CONFIG_GLOBAL", &self.world.gitconfig)
            .env("HERDR_SESSION_DRAIN_INLINE", "1")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let mut stdin = child.stdin.take().unwrap();
        let mut send = |v: Value| writeln!(stdin, "{v}").unwrap();
        send(serde_json::json!({ "jsonrpc": "2.0", "id": 0, "method": "initialize", "params": { "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "test", "version": "0" } } }));
        send(serde_json::json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }));
        for (i, (name, args)) in calls.iter().enumerate() {
            send(serde_json::json!({ "jsonrpc": "2.0", "id": i + 1, "method": "tools/call", "params": { "name": name, "arguments": args } }));
        }
        drop(send);
        drop(stdin);
        let out = child.wait_with_output().unwrap();
        assert!(out.status.success());
        let replies: Vec<Value> = String::from_utf8_lossy(&out.stdout).lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        assert_eq!(replies.len(), calls.len() + 1, "one reply per request, none for the notification");
        assert_eq!(replies[0]["result"]["serverInfo"]["name"], "herdr-live-share");
        replies[1..]
            .iter()
            .map(|r| (r["result"]["content"][0]["text"].as_str().unwrap_or_default().to_string(), r["result"]["isError"].as_bool().unwrap_or(false)))
            .collect()
    }

    /// The text Claude is given (additionalContext), or "" when the hook was silent.
    pub fn context(&self, dir: &Path, event: &str, input: Value) -> String {
        let out = self.hook(dir, event, input);
        if out.trim().is_empty() {
            return String::new();
        }
        let v: Value = serde_json::from_str(out.trim()).expect("hook output is JSON");
        v["hookSpecificOutput"]["additionalContext"].as_str().unwrap_or_default().to_string()
    }

    /// git as this person, with the world's GitHub.
    pub fn git(&self, dir: &Path, args: &[&str]) -> String {
        let out = Command::new("git").arg("-C").arg(dir).args(args)
            .env("HOME", &self.home)
            .env("GIT_CONFIG_GLOBAL", &self.world.gitconfig)
            .env("GIT_AUTHOR_NAME", &self.login).env("GIT_AUTHOR_EMAIL", "t@t").env("GIT_COMMITTER_NAME", &self.login).env("GIT_COMMITTER_EMAIL", "t@t")
            .output().unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    pub fn ok(&self, dir: &Path, args: &[&str]) -> Value {
        let v = self.cli(dir, args, &[]);
        assert_eq!(v["ok"], true, "{args:?}: {v}");
        v["value"].clone()
    }
    pub fn fails(&self, args: &[&str], gh: &[(&str, &str)]) -> (String, String) {
        let v = self.cli(&self.home, args, gh);
        assert_eq!(v["ok"], false, "{args:?} should fail: {v}");
        (v["error"]["kind"].as_str().unwrap().into(), v["error"]["message"].as_str().unwrap().into())
    }
    pub fn join(&self, link: &str, step: &str) -> Value {
        self.ok(&self.home, &["join", "--link", link, "--step", step])
    }
}

/// Thomas shares google-search; returns the link.
pub fn share(w: &World) -> (Person<'_>, PathBuf, String) {
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

