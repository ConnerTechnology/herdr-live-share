//! Pushing after an agent's turn, in a detached child so the turn never waits
//! on the network. One drain per clone at a time (a file lock); the next
//! prompt waits for it, so an agent never starts editing mid-rebase.
//!
//! Only commits are pushed: drain never commits for anyone. Pushes made any
//! other way (a terminal, an IDE) are noticed from git's own record of them
//! (the remote-tracking ref's reflog says "update by push") and become the
//! same `Pushed` event.

use crate::git::{Checkout, GitError};
use crate::ledger::{Body, CommitLine, Ms};
use crate::session::{self, Binding, Error};
use crate::util::now_ms;
use serde::Serialize;
use std::fs::File;
use std::path::PathBuf;
use std::time::{Duration, Instant};

const NET: Duration = Duration::from_secs(60);

fn lock_path(at: &Checkout) -> PathBuf {
    at.common_dir.join("herdr-session").join("drain.lock")
}

fn lock_file(at: &Checkout) -> std::io::Result<File> {
    let path = lock_path(at);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    File::options().create(true).truncate(false).write(true).open(path)
}

/// Wait (at most `max`) for a running drain to finish. True when none is running.
pub fn wait_idle(at: &Checkout, max: Duration) -> bool {
    let Ok(f) = lock_file(at) else { return true };
    let until = Instant::now() + max;
    loop {
        if f.try_lock().is_ok() {
            let _ = f.unlock();
            return true;
        }
        if Instant::now() >= until {
            return false;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Outcome {
    /// Nothing to push (or Keep in sync is off).
    Nothing,
    Pushed { commits: usize, to: String },
    /// The rebase stopped on these files; it was undone, the commits are still here.
    Conflict { files: Vec<String> },
}

/// Push the branch (when Keep in sync is on and there is something to push),
/// notice pushes made elsewhere, then sync the ledger. `ledger_only` skips the branch.
pub fn drain(at: &Checkout, b: &Binding, ledger_only: bool) -> Result<Outcome, Error> {
    let f = lock_file(at)?;
    f.lock()?;
    let mut outcome = Outcome::Nothing;
    if !b.read_only {
        if !ledger_only && b.keep_in_sync {
            outcome = push_branch(at, b)?;
        }
        notice_push(at, b, now_ms())?;
    }
    let net = session::sync(at, b, NET);
    let _ = f.unlock();
    if let Some(e) = net.last_error {
        // The branch push already happened (or not); the ledger goes with the next sync.
        eprintln!("ledger sync: {e}");
    }
    Ok(outcome)
}

fn upstream(b: &Binding, at: &Checkout) -> String {
    format!("refs/remotes/{}/{}", b.remote, at.branch)
}

fn commits(at: &Checkout, range: &[&str]) -> Result<Vec<CommitLine>, GitError> {
    let mut args = vec!["log", "--reverse", "--format=%H%x09%s"];
    args.extend_from_slice(range);
    Ok(at
        .repo()
        .run(&args)?
        .lines()
        .filter_map(|l| l.split_once('\t'))
        .map(|(oid, subject)| CommitLine { oid: oid.into(), subject: subject.into() })
        .collect())
}

fn summary(commits: &[CommitLine]) -> String {
    let s = commits.iter().map(|c| c.subject.as_str()).collect::<Vec<_>>().join("; ");
    if s.chars().count() > 200 { format!("{}…", s.chars().take(199).collect::<String>()) } else { s }
}

fn push_branch(at: &Checkout, b: &Binding) -> Result<Outcome, Error> {
    let repo = at.repo();
    let up = upstream(b, at);
    for _ in 0..3 {
        repo.fetch(&b.remote, &format!("+refs/heads/{0}:{up}", at.branch), NET)?;
        let count = |range: &str| -> Result<u64, GitError> { Ok(repo.run(&["rev-list", "--count", range])?.trim().parse().unwrap_or(0)) };
        if count(&format!("{up}..HEAD"))? == 0 {
            return Ok(Outcome::Nothing);
        }
        if count(&format!("HEAD..{up}"))? > 0 {
            if let Err(e) = repo.run(&["-c", "rebase.autoStash=true", "rebase", "--quiet", &up]) {
                let files: Vec<String> = repo.run(&["diff", "--name-only", "--diff-filter=U"]).unwrap_or_default().lines().map(str::to_string).collect();
                let _ = repo.run(&["rebase", "--abort"]);
                if files.is_empty() {
                    return Err(e.into());
                }
                session::append(at, b, Body::Conflict { files: files.clone() }, None, now_ms())?;
                return Ok(Outcome::Conflict { files });
            }
        }
        let pushed = commits(at, &[&format!("{up}..HEAD")])?;
        match repo.push(&b.remote, &format!("HEAD:refs/heads/{}", at.branch), NET) {
            Ok(()) => {
                let to = repo.run(&["rev-parse", "HEAD"])?.trim().to_string();
                let body = Body::Pushed { summary: summary(&pushed), commits: pushed.clone(), to: Some(to.clone()) };
                session::append(at, b, body, None, now_ms())?;
                set_noticed(at, b, &to);
                return Ok(Outcome::Pushed { commits: pushed.len(), to });
            }
            // Someone pushed in between: fetch, rebase and try again.
            Err(GitError::Failed(m)) if m.contains("non-fast-forward") || m.contains("rejected") || m.contains("fetch first") => continue,
            Err(e) => return Err(e.into()),
        }
    }
    Err(Error::Git(GitError::Failed("the branch kept moving on GitHub; try again".into())))
}

fn noticed_path(at: &Checkout, b: &Binding) -> PathBuf {
    at.common_dir.join("herdr-session").join(b.sid().code()).join(format!("noticed-push-{}", at.branch.replace('/', "%2F")))
}

fn set_noticed(at: &Checkout, b: &Binding, oid: &str) {
    let _ = session::write_atomic(&noticed_path(at, b), oid.as_bytes());
}

/// A push this machine made outside herdr: the newest "update by push" in the
/// upstream's reflog that the ledger doesn't have yet. Local only.
pub fn notice_push(at: &Checkout, b: &Binding, now: Ms) -> Result<(), Error> {
    if b.read_only {
        return Ok(());
    }
    let repo = at.repo();
    let Ok(log) = repo.run(&["reflog", "show", "--format=%H%x09%gs", &upstream(b, at)]) else { return Ok(()) };
    let entries: Vec<(&str, &str)> = log.lines().filter_map(|l| l.split_once('\t')).collect();
    let Some(i) = entries.iter().position(|(_, why)| why.starts_with("update by push")) else { return Ok(()) };
    let to = entries[i].0;
    let noticed = std::fs::read_to_string(noticed_path(at, b)).unwrap_or_default();
    if noticed.trim() == to {
        return Ok(());
    }
    set_noticed(at, b, to);
    let f = session::load(at, b, now)?;
    let known = f.items.iter().any(|it| matches!(&it.kind, crate::ledger::ItemKind::Authored(Body::Pushed { to: Some(t), .. }) if t == to));
    if known {
        return Ok(());
    }
    // The commits it added: from the value before that push (the next older reflog entry).
    let range: Vec<String> = match entries.get(i + 1) {
        Some((from, _)) => vec![format!("{from}..{to}")],
        None => vec!["-1".into(), to.to_string()],
    };
    let range: Vec<&str> = range.iter().map(String::as_str).collect();
    let pushed = commits(at, &range)?;
    if pushed.is_empty() {
        return Ok(());
    }
    session::append(at, b, Body::Pushed { summary: summary(&pushed), commits: pushed, to: Some(to.into()) }, None, now)?;
    Ok(())
}

/// Run `herdr-session drain` detached: it outlives the hook that started it.
/// $HERDR_SESSION_DRAIN_INLINE=1 runs it in place (tests).
pub fn spawn(at: &Checkout, ledger_only: bool) {
    let root = at.root.to_string_lossy().to_string();
    let mut args = vec!["drain".to_string(), "--cwd".into(), root];
    if ledger_only {
        args.push("--ledger".into());
    }
    if std::env::var_os("HERDR_SESSION_DRAIN_INLINE").is_some() {
        if let Some(b) = session::binding(at) {
            let _ = drain(at, &b, ledger_only);
        }
        return;
    }
    crate::util::spawn_detached(&args, &session::home().join("logs").join("drain.log"));
    crate::listen::ensure(at);
}
