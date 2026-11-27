//! Joining through the real binary. "GitHub" is a local bare repo that git
//! reaches through a global url.insteadOf, and `gh` is a fake on PATH that
//! answers from environment variables, so every join failure can be staged.

mod common;

use common::{git, share, World};
use std::path::PathBuf;

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
