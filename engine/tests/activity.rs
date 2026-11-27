//! The Activity window's timeline (SPEC 2.6, 10), read from both sides of a session.

mod common;

use common::{share, World};
use serde_json::{json, Value};

#[test]
fn the_timeline_shows_the_session_and_the_agents_conversation() {
    let w = World::new();
    let (thomas, tdir, link) = share(&w);
    let ryan = w.person("ryan");
    ryan.join(&link, "branch");
    let rdir = ryan.home.join("herdr-sessions/joindeleterepo/google-search");

    // Ryan's agent says what it is doing and asks Thomas's; Thomas's answers.
    let asked = ryan.mcp(&rdir, "R1", &[
        ("share_post", json!({ "kind": "working", "text": "Making drain retry a rejected push once." })),
        ("share_ask", json!({ "to": "thomas", "text": "Is query.py safe to edit?" })),
    ]);
    assert!(!asked[1].1, "{:?}", asked[1]);
    thomas.mcp(&tdir, "T1", &[("share_answer", json!({ "question": "query.py", "text": "Yes, I'm done with it." }))]);

    // Thomas pushes a commit after his agent's turn.
    std::fs::write(tdir.join("query.py"), "def build():\n    return 2\n").unwrap();
    thomas.git(&tdir, &["commit", "-qam", "Return 2"]);
    thomas.hook(&tdir, "stop", json!({ "session_id": "T1", "cwd": tdir, "last_assistant_message": "Returned 2." }));
    let oid = thomas.git(&tdir, &["rev-parse", "HEAD"]).trim().to_string();

    // Thomas's window, after a sync.
    let v = thomas.ok(&tdir, &["timeline", "--sync"]);
    let t = &v["timeline"];
    assert_eq!((t["name"].as_str(), t["branch"].as_str(), t["open"].as_bool(), t["me"].as_str()), (Some("Google Search"), Some("google-search"), Some(true), Some("thomas")));
    assert_eq!(t["members"], json!(["thomas", "ryan"]), "owner first, then by join");
    assert!(v["net"]["last_ok"].is_number(), "{v}");

    let entries = t["entries"].as_array().unwrap();
    let ats: Vec<u64> = entries.iter().map(|e| e["at"].as_u64().unwrap()).collect();
    assert!(ats.windows(2).all(|p| p[0] >= p[1]), "newest first: {ats:?}");
    let find = |verb: &str| -> &Value { entries.iter().find(|e| e["verb"] == verb).unwrap_or_else(|| panic!("no `{verb}` in {entries:#?}")) };

    let q = find("asked you");
    assert_eq!((q["login"].as_str(), q["agent"].as_bool(), q["label"].as_str(), q["quote"].as_str()), (Some("ryan"), Some(true), Some("Question"), Some("Is query.py safe to edit?")));
    assert_eq!(q["answers"].as_array().unwrap().len(), 1);
    let a = &q["answers"][0];
    assert_eq!((a["login"].as_str(), a["agent"].as_bool(), a["text"].as_str(), a["background"].as_bool()), (Some("thomas"), Some(true), Some("Yes, I'm done with it."), Some(false)));
    assert!(entries.iter().all(|e| !e["verb"].as_str().unwrap().contains("answer")), "answers sit under their question, not on their own");

    let post = find("is working on");
    assert_eq!((post["label"].as_str(), post["quote"].as_str()), (Some("Working on"), Some("Making drain retry a rejected push once.")));

    let push = find("pushed 1 commit");
    assert_eq!(push["quote"], "Return 2");
    assert_eq!(push["link"], format!("https://github.com/deleteme/joindeleterepo/commit/{oid}"));
    assert_eq!(find("finished a turn")["quote"], "Returned 2.");
    assert_eq!(find("joined")["login"], "ryan");
    assert_eq!(find("started the session")["login"], "thomas");

    // Ryan's window names the person he asked.
    let v = ryan.ok(&rdir, &["timeline", "--sync"]);
    let entries = v["timeline"]["entries"].as_array().unwrap();
    assert!(entries.iter().any(|e| e["verb"] == "asked thomas" && e["answers"].as_array().unwrap().len() == 1), "{entries:#?}");

    // Not shared: a plain checkout says so.
    let plain = w.root.join("plain");
    std::fs::create_dir(&plain).unwrap();
    common::git(&plain, &["init", "-q", "-b", "main"]);
    common::git(&plain, &["commit", "-q", "--allow-empty", "-m", "init"]);
    assert_eq!(thomas.cli(&plain, &["timeline"], &[])["error"]["kind"], "NotShared");
}
