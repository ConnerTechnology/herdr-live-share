//! Two people's Claude agents in one shared session, through the real hooks:
//! a heads-up before editing the same file, turn summaries, pushes after a
//! turn, a rebase conflict, and a push made from a terminal.

mod common;

use common::{share, World};
use serde_json::json;
use std::path::Path;

fn input(dir: &Path, session: &str) -> serde_json::Value {
    json!({ "session_id": session, "cwd": dir, "hook_event_name": "x" })
}

fn edit(dir: &Path, session: &str, file: &str) -> serde_json::Value {
    json!({ "session_id": session, "cwd": dir, "tool_name": "Edit", "tool_input": { "file_path": dir.join(file) } })
}

fn said(dir: &Path, session: &str, text: &str) -> serde_json::Value {
    json!({ "session_id": session, "cwd": dir, "last_assistant_message": text })
}

#[test]
fn two_agents_work_on_one_branch() {
    let w = World::new();
    let (thomas, tdir, link) = share(&w);
    let ryan = w.person("ryan");
    ryan.join(&link, "branch");
    let rdir = ryan.home.join("herdr-sessions/joindeleterepo/google-search");

    // Both agents start and catch up.
    thomas.context(&tdir, "start", input(&tdir, "T1"));
    assert!(ryan.context(&rdir, "start", input(&rdir, "R1")).contains("Caught up"));
    assert_eq!(thomas.context(&tdir, "prompt", input(&tdir, "T1")), "", "nothing new is silent");

    // Ryan's agent starts on query.py: no heads-up for him, and Thomas's agent is warned.
    assert_eq!(ryan.context(&rdir, "pre-edit", edit(&rdir, "R1", "query.py")), "");
    let heads_up = thomas.context(&tdir, "pre-edit", edit(&tdir, "T1", "query.py"));
    assert!(heads_up.contains("ryan's agent is working on query.py"), "{heads_up}");
    assert_eq!(thomas.context(&tdir, "pre-edit", edit(&tdir, "T1", "notes.md")), "", "another file: no heads-up");

    // Ryan's turn ends: his summary and his commit reach GitHub.
    std::fs::write(rdir.join("ranking.py"), "boost = 0.2\n").unwrap();
    ryan.git(&rdir, &["add", "."]);
    ryan.git(&rdir, &["commit", "-q", "-m", "Boost fresh results"]);
    ryan.hook(&rdir, "stop", said(&rdir, "R1", "**Boosted ranking for fresh results.**\n\nDetails follow."));
    let tip = ryan.git(&rdir, &["rev-parse", "HEAD"]);
    assert_eq!(common::git(&w.remote, &["rev-parse", "google-search"]), tip, "pushed after the turn");

    // Thomas's agent hears both on its next prompt, once.
    let news = thomas.context(&tdir, "prompt", input(&tdir, "T1"));
    assert!(news.contains("ryan pushed 1 commit: \"Boost fresh results\""), "{news}");
    assert!(news.contains("ryan's agent: \"Boosted ranking for fresh results.\""), "{news}");
    assert_eq!(thomas.context(&tdir, "prompt", input(&tdir, "T1")), "");

    // Thomas changes the same line without pulling: his push stops on a conflict, undone, and his agent is told.
    std::fs::write(tdir.join("query.py"), "def build():\n    return 2\n").unwrap();
    thomas.git(&tdir, &["commit", "-qam", "Return 2"]);
    std::fs::write(rdir.join("query.py"), "def build():\n    return 3\n").unwrap();
    ryan.git(&rdir, &["commit", "-qam", "Return 3"]);
    ryan.hook(&rdir, "stop", said(&rdir, "R1", "Return 3 now."));
    let mine = thomas.git(&tdir, &["rev-parse", "HEAD"]);
    thomas.hook(&tdir, "stop", said(&tdir, "T1", "Return 2 now."));
    assert_eq!(thomas.git(&tdir, &["rev-parse", "HEAD"]), mine, "the rebase was undone; Thomas's commit is intact");
    assert!(thomas.git(&tdir, &["status", "--porcelain"]).is_empty(), "no half-finished rebase left behind");
    let told = thomas.context(&tdir, "prompt", input(&tdir, "T1"));
    assert!(told.contains("thomas hit a rebase conflict in query.py; nothing was pushed"), "{told}");
    assert!(ryan.context(&rdir, "prompt", input(&rdir, "R1")).contains("thomas hit a rebase conflict in query.py"));

    // A push from Ryan's terminal is noticed once and told to Thomas once.
    std::fs::write(rdir.join("README.md"), "search\n").unwrap();
    ryan.git(&rdir, &["add", "."]);
    ryan.git(&rdir, &["commit", "-qm", "Add a README"]);
    ryan.git(&rdir, &["push", "-q"]);
    ryan.context(&rdir, "prompt", input(&rdir, "R1"));
    ryan.context(&rdir, "prompt", input(&rdir, "R2"));
    let news = thomas.context(&tdir, "prompt", input(&tdir, "T1"));
    assert_eq!(news.matches("Add a README").count(), 1, "{news}");
    assert!(news.contains("ryan pushed 1 commit: \"Add a README\""), "{news}");

    // Keep in sync off: Ryan's commits stay on his machine.
    ryan.ok(&rdir, &["change", "--keep-in-sync", "off"]);
    let before = common::git(&w.remote, &["rev-parse", "google-search"]);
    std::fs::write(rdir.join("local.txt"), "x\n").unwrap();
    ryan.git(&rdir, &["add", "."]);
    ryan.git(&rdir, &["commit", "-qm", "Local only"]);
    ryan.hook(&rdir, "stop", said(&rdir, "R1", "Kept it local."));
    assert_eq!(common::git(&w.remote, &["rev-parse", "google-search"]), before);
    assert!(thomas.context(&tdir, "prompt", input(&tdir, "T1")).contains("Kept it local."), "the summary still goes out");
}

#[test]
fn hooks_stay_silent_outside_shared_checkouts() {
    let w = World::new();
    let p = w.person("priya");
    let dir = w.root.join("plain");
    std::fs::create_dir(&dir).unwrap();
    common::git(&dir, &["init", "-q", "-b", "main"]);
    for event in ["start", "prompt", "pre-edit", "stop", "end"] {
        assert_eq!(p.hook(&dir, event, edit(&dir, "P1", "a.txt")), "", "{event}");
    }
    assert_eq!(p.hook(&w.root, "prompt", input(&w.root, "P1")), "", "not a repo");
}

#[test]
fn one_person_with_two_checkouts_of_a_session() {
    // Thomas shares from his folder, then joins his own session in a second checkout on the same machine.
    let w = World::new();
    let (thomas, tdir, link) = share(&w);
    thomas.join(&link, "branch");
    let second = thomas.home.join("herdr-sessions/joindeleterepo/google-search");

    thomas.context(&tdir, "start", input(&tdir, "T1"));
    assert_eq!(thomas.context(&second, "pre-edit", edit(&second, "T2", "query.py")), "");
    std::fs::write(second.join("query.py"), "def build():\n    return 2\n").unwrap();
    thomas.git(&second, &["commit", "-qam", "Return 2"]);
    thomas.hook(&second, "stop", said(&second, "T2", "Returned 2."));

    // The first checkout's agent hears the other one, and both reach GitHub without a fight.
    let news = thomas.context(&tdir, "prompt", input(&tdir, "T1"));
    assert!(!news.contains("Can't reach"), "{news}");
    assert!(news.contains("thomas pushed 1 commit: \"Return 2\""), "{news}");
    assert!(news.contains("thomas's agent: \"Returned 2.\""), "{news}");
    thomas.hook(&tdir, "stop", said(&tdir, "T1", "Read the news."));
    assert!(!thomas.context(&second, "prompt", input(&second, "T2")).contains("Can't reach"));
}
