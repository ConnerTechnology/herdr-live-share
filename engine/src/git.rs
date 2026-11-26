//! Shell-outs to the `git` binary, so the user's credentials and config apply.
//! Ledger streams are commit chains: each commit holds one blob `e` (the
//! signed event); author and date are fixed so the same event hashes the same.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitError {
    Failed(String),
    Timeout,
}

impl std::fmt::Display for GitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GitError::Failed(m) => write!(f, "{m}"),
            GitError::Timeout => write!(f, "git timed out"),
        }
    }
}

/// A repository: any directory inside a checkout or a bare clone.
#[derive(Debug, Clone)]
pub struct Repo {
    pub dir: PathBuf,
}

/// Where a hook or a mod is working: the checkout's root, its shared git dir and branch.
#[derive(Debug, Clone)]
pub struct Checkout {
    pub root: PathBuf,
    pub common_dir: PathBuf,
    pub branch: String,
}

impl Checkout {
    /// None outside a git checkout or on a detached HEAD.
    pub fn discover(cwd: &Path) -> Option<Checkout> {
        let r = Repo { dir: cwd.to_path_buf() };
        let out = r.run(&["rev-parse", "--show-toplevel", "--path-format=absolute", "--git-common-dir", "--abbrev-ref", "HEAD"]).ok()?;
        let mut lines = out.lines();
        let root = PathBuf::from(lines.next()?);
        let common_dir = PathBuf::from(lines.next()?);
        let branch = lines.next()?.to_string();
        if branch == "HEAD" {
            return None;
        }
        Some(Checkout { root, common_dir, branch })
    }

    pub fn repo(&self) -> Repo {
        Repo { dir: self.root.clone() }
    }
}

impl Repo {
    pub fn run(&self, args: &[&str]) -> Result<String, GitError> {
        self.run_with(args, None, None)
    }

    /// Run git with optional stdin and a deadline; stdout on success.
    pub fn run_with(&self, args: &[&str], stdin: Option<&[u8]>, timeout: Option<Duration>) -> Result<String, GitError> {
        let mut child = Command::new("git")
            .arg("-C")
            .arg(&self.dir)
            .args(args)
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_AUTHOR_NAME", "herdr")
            .env("GIT_AUTHOR_EMAIL", "herdr@invalid")
            .env("GIT_AUTHOR_DATE", "@0 +0000")
            .env("GIT_COMMITTER_NAME", "herdr")
            .env("GIT_COMMITTER_EMAIL", "herdr@invalid")
            .env("GIT_COMMITTER_DATE", "@0 +0000")
            .stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| GitError::Failed(format!("git: {e}")))?;
        if let (Some(input), Some(mut pipe)) = (stdin, child.stdin.take()) {
            let input = input.to_vec();
            std::thread::spawn(move || {
                let _ = pipe.write_all(&input);
            });
        }
        let mut out_pipe = child.stdout.take().unwrap();
        let mut err_pipe = child.stderr.take().unwrap();
        let out_t = std::thread::spawn(move || {
            let mut b = Vec::new();
            let _ = out_pipe.read_to_end(&mut b);
            b
        });
        let err_t = std::thread::spawn(move || {
            let mut b = Vec::new();
            let _ = err_pipe.read_to_end(&mut b);
            b
        });
        let deadline = timeout.map(|t| Instant::now() + t);
        let status = loop {
            match child.try_wait() {
                Ok(Some(s)) => break s,
                Ok(None) => {}
                Err(e) => return Err(GitError::Failed(e.to_string())),
            }
            if deadline.is_some_and(|d| Instant::now() >= d) {
                let _ = child.kill();
                let _ = child.wait();
                return Err(GitError::Timeout);
            }
            std::thread::sleep(Duration::from_millis(5));
        };
        let out = out_t.join().unwrap_or_default();
        let err = err_t.join().unwrap_or_default();
        if status.success() {
            Ok(String::from_utf8_lossy(&out).into_owned())
        } else {
            Err(GitError::Failed(String::from_utf8_lossy(&err).trim().to_string()))
        }
    }

    /// Refs under a prefix: (name, commit).
    pub fn refs(&self, prefix: &str) -> Result<Vec<(String, String)>, GitError> {
        let out = self.run(&["for-each-ref", "--format=%(refname) %(objectname)", prefix])?;
        Ok(out.lines().filter_map(|l| l.split_once(' ')).map(|(n, o)| (n.to_string(), o.to_string())).collect())
    }

    pub fn ref_oid(&self, name: &str) -> Option<String> {
        self.run(&["rev-parse", "-q", "--verify", &format!("{name}^{{commit}}")]).ok().map(|s| s.trim().to_string())
    }

    /// Every event blob in a stream, seq 0 first.
    pub fn read_stream(&self, name: &str) -> Result<Vec<Vec<u8>>, GitError> {
        let commits = self.run(&["rev-list", "--reverse", "--first-parent", name])?;
        let wanted: String = commits.lines().map(|c| format!("{c}:e\n")).collect();
        if wanted.is_empty() {
            return Ok(Vec::new());
        }
        let raw = self.cat_batch(wanted.as_bytes())?;
        let mut blobs = Vec::new();
        let mut rest = &raw[..];
        while !rest.is_empty() {
            let nl = rest.iter().position(|&b| b == b'\n').ok_or_else(|| GitError::Failed("bad cat-file output".into()))?;
            let header = String::from_utf8_lossy(&rest[..nl]).to_string();
            rest = &rest[nl + 1..];
            let mut parts = header.split(' ');
            let (_, kind, size) = (parts.next(), parts.next(), parts.next());
            if kind != Some("blob") {
                break; // a commit without `e`: the stream ends here
            }
            let size: usize = size.and_then(|s| s.parse().ok()).ok_or_else(|| GitError::Failed("bad size".into()))?;
            blobs.push(rest[..size].to_vec());
            rest = &rest[size + 1..];
        }
        Ok(blobs)
    }

    fn cat_batch(&self, input: &[u8]) -> Result<Vec<u8>, GitError> {
        let mut child = Command::new("git")
            .arg("-C")
            .arg(&self.dir)
            .args(["cat-file", "--batch"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| GitError::Failed(e.to_string()))?;
        let mut pipe = child.stdin.take().unwrap();
        let input = input.to_vec();
        let w = std::thread::spawn(move || {
            let _ = pipe.write_all(&input);
        });
        let mut out = Vec::new();
        child.stdout.take().unwrap().read_to_end(&mut out).map_err(|e| GitError::Failed(e.to_string()))?;
        let _ = w.join();
        child.wait().map_err(|e| GitError::Failed(e.to_string()))?;
        Ok(out)
    }

    /// Append one blob to a stream. `old` is the tip we built on (None: a new
    /// stream); the update fails if the ref moved, so two writers can't fork it.
    pub fn append(&self, name: &str, old: Option<&str>, blob: &[u8]) -> Result<String, GitError> {
        let blob_oid = self.run_with(&["hash-object", "-w", "--stdin"], Some(blob), None)?.trim().to_string();
        let tree = self.run_with(&["mktree"], Some(format!("100644 blob {blob_oid}\te\n").as_bytes()), None)?.trim().to_string();
        let mut args = vec!["commit-tree", tree.as_str(), "-m", "herdr"];
        if let Some(p) = old {
            args.push("-p");
            args.push(p);
        }
        let commit = self.run(&args)?.trim().to_string();
        let zero = "0000000000000000000000000000000000000000";
        self.run(&["update-ref", name, &commit, old.unwrap_or(zero)])?;
        Ok(commit)
    }

    pub fn fetch(&self, remote: &str, refspec: &str, timeout: Duration) -> Result<(), GitError> {
        self.run_with(&["fetch", "--quiet", "--no-tags", remote, refspec], None, Some(timeout)).map(|_| ())
    }

    pub fn push(&self, remote: &str, refspec: &str, timeout: Duration) -> Result<(), GitError> {
        self.run_with(&["push", "--quiet", "--no-verify", remote, refspec], None, Some(timeout)).map(|_| ())
    }

    pub fn remote_url(&self, remote: &str) -> Option<String> {
        self.run(&["remote", "get-url", remote]).ok().map(|s| s.trim().to_string())
    }
}
