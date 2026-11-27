use super::*;
use proptest::prelude::*;

const H: Ms = 3_600_000;
const DAY: Ms = 24 * H;
const T0: Ms = 1_764_000_000_000;

fn key(n: u8) -> DeviceKey {
    DeviceKey::from_seed([n; 32])
}

fn settings(auto_close: bool) -> Settings {
    Settings { name: "Google Search".into(), auto_close, who_can_join: WhoCanJoin::Anyone }
}

fn genesis(auto_close: bool) -> Genesis {
    Genesis {
        repo: "https://github.com/deleteme/joindeleterepo".into(),
        branch: "google-search".into(),
        owner: "thomas".into(),
        created: T0,
        nonce: "00".into(),
        settings: settings(auto_close),
    }
}

/// A device writing a stream.
struct Dev {
    key: DeviceKey,
    login: String,
    sid: SessionId,
    blobs: Vec<Vec<u8>>,
}

impl Dev {
    fn new(n: u8, login: &str, sid: SessionId) -> Self {
        Dev { key: key(n), login: login.into(), sid, blobs: vec![] }
    }
    fn say(&mut self, at: Ms, body: Body) -> &mut Self {
        let e = Event { sid: self.sid.hex(), seq: self.blobs.len() as u64, at, login: self.login.clone(), agent: None, body };
        self.blobs.push(self.key.sign(&e));
        self
    }
    fn stream(&self) -> Verified {
        verify(&self.sid, &self.key.tag(&self.sid), &self.blobs)
    }
    fn prefix(&self, n: usize) -> Verified {
        verify(&self.sid, &self.key.tag(&self.sid), &self.blobs[..n])
    }
}

/// Thomas opens the session; Ryan has joined.
fn pair(auto_close: bool) -> (Dev, Dev) {
    let g = genesis(auto_close);
    let sid = g.session_id();
    let mut thomas = Dev::new(1, "thomas", sid);
    thomas.say(T0, Body::Opened { genesis: g });
    let mut ryan = Dev::new(2, "ryan", sid);
    ryan.say(T0 + H, Body::Joined);
    (thomas, ryan)
}

#[test]
fn code_is_eight_base32_chars() {
    let code = genesis(true).session_id().code();
    assert_eq!(code.len(), 8);
    assert!(code.chars().all(|c| c.is_ascii_lowercase() || ('2'..='7').contains(&c)));
}

#[test]
fn verify_keeps_the_valid_prefix() {
    let (mut thomas, _) = pair(true);
    thomas.say(T0 + 1, Body::Chat { text: "hi".into() }).say(T0 + 2, Body::Chat { text: "two".into() });
    let mut blobs = thomas.blobs.clone();
    blobs[1][10] ^= 1; // tamper with seq 1
    let v = verify(&thomas.sid, &thomas.key.tag(&thomas.sid), &blobs);
    assert_eq!(v.events().len(), 1, "seq 1 is forged, so it and everything after it go");
}

#[test]
fn verify_rejects_a_stream_under_another_devices_tag() {
    let (thomas, ryan) = pair(true);
    let v = verify(&thomas.sid, &ryan.key.tag(&ryan.sid), &thomas.blobs);
    assert!(v.events().is_empty());
}

#[test]
fn verify_rejects_a_seq_gap() {
    let (thomas, _) = pair(true);
    let e = Event { sid: thomas.sid.hex(), seq: 2, at: T0, login: "thomas".into(), agent: None, body: Body::Closed };
    let blobs = vec![thomas.blobs[0].clone(), thomas.key.sign(&e)];
    assert_eq!(verify(&thomas.sid, &thomas.key.tag(&thomas.sid), &blobs).events().len(), 1);
}

#[test]
fn an_unknown_event_kind_is_skipped_not_fatal() {
    let (thomas, mut ryan) = pair(true);
    // A newer release writes a kind this one does not know, then a known one.
    let line = format!(
        r#"{{"sid":"{}","seq":1,"at":{},"login":"ryan","body":{{"kind":"poll","question":"lunch?"}}}}"#,
        ryan.sid.hex(),
        T0 + 2 * H
    );
    let sig = ryan.key.0.sign(line.as_bytes()).to_bytes();
    ryan.blobs.push(blob(&ryan.key.pk(), line.as_bytes(), &sig));
    ryan.say(T0 + 3 * H, Body::Chat { text: "after".into() });
    let v = ryan.stream();
    assert_eq!(v.events().len(), 3, "the stream stays whole");
    assert!(matches!(&v.events()[1].event.body, Body::Unknown { kind } if kind == "poll"));
    let f = fold(&thomas.sid, &[thomas.stream(), v], T0 + 4 * H).unwrap();
    let chats = f.items.iter().filter(|i| matches!(&i.kind, ItemKind::Authored(Body::Chat { .. }))).count();
    assert_eq!(chats, 1);
    assert!(!f.items.iter().any(|i| matches!(&i.kind, ItemKind::Authored(Body::Unknown { .. }))));
}

#[test]
fn a_genesis_from_a_newer_release_keeps_its_id() {
    // A newer release adds a settings field; this release must still find the session.
    let line_genesis = serde_json::json!({
        "repo": "r", "branch": "b", "owner": "thomas", "created": T0, "nonce": "00",
        "settings": { "name": "S", "auto_close": true, "who_can_join": { "who": "anyone" }, "theme": "violet" }
    });
    let sid = genesis_id(&line_genesis);
    let k = key(1);
    let line = serde_json::to_vec(&serde_json::json!({
        "sid": sid.hex(), "seq": 0, "at": T0, "login": "thomas",
        "body": { "kind": "opened", "genesis": line_genesis }
    }))
    .unwrap();
    let sig = k.0.sign(&line).to_bytes();
    let v = verify(&sid, &k.tag(&sid), &[blob(&k.pk(), &line, &sig)]);
    let f = fold(&sid, &[v], T0).expect("found despite the unknown field");
    assert_eq!(f.settings.name, "S");
}

#[test]
fn no_genesis_no_fold() {
    let (_, ryan) = pair(true);
    assert!(fold(&ryan.sid, &[ryan.stream()], T0).is_none());
}

#[test]
fn members_and_owner_only_settings() {
    let (mut thomas, mut ryan) = pair(true);
    ryan.say(T0 + 2 * H, Body::SettingsChanged { patch: SettingsPatch::Name { name: "Ryan's now".into() } });
    ryan.say(T0 + 2 * H, Body::Closed);
    thomas.say(T0 + 3 * H, Body::SettingsChanged { patch: SettingsPatch::Name { name: "Search v2".into() } });
    let f = fold(&thomas.sid, &[thomas.stream(), ryan.stream()], T0 + 4 * H).unwrap();
    assert_eq!(f.settings.name, "Search v2", "only the owner renames");
    assert!(f.is_open(), "only the owner closes");
    assert_eq!(f.members.keys().cloned().collect::<Vec<_>>(), vec!["ryan".to_string(), "thomas".to_string()]);
    assert_eq!(f.members["ryan"].joined, T0 + H);
}

#[test]
fn empty_only_list_means_anyone() {
    let (mut thomas, _) = pair(true);
    thomas.say(T0 + 1, Body::SettingsChanged { patch: SettingsPatch::WhoCanJoin { who: WhoCanJoin::Only { logins: BTreeSet::new() } } });
    let f = fold(&thomas.sid, &[thomas.stream()], T0 + 2).unwrap();
    assert_eq!(f.settings.who_can_join, WhoCanJoin::Anyone);
}

#[test]
fn auto_close_after_14_idle_days_and_closing_soon_before() {
    let (thomas, ryan) = pair(true);
    let streams = [thomas.stream(), ryan.stream()];
    let last = T0 + H;

    let f = fold(&thomas.sid, &streams, last + 11 * DAY).unwrap();
    assert_eq!(f.status, Status::Open { closes_at: Some(last + IDLE_CLOSE) });
    assert!(!f.items.iter().any(|i| matches!(i.kind, ItemKind::ClosingSoon { .. })));

    let f = fold(&thomas.sid, &streams, last + 13 * DAY).unwrap();
    assert!(f.items.iter().any(|i| matches!(i.kind, ItemKind::ClosingSoon { .. })));

    let f = fold(&thomas.sid, &streams, last + 15 * DAY).unwrap();
    assert_eq!(f.status, Status::Closed { at: last + IDLE_CLOSE, why: CloseWhy::Idle });
    assert!(f.items.iter().any(|i| i.kind == ItemKind::AutoClosed));

    let (thomas, ryan) = pair(false);
    let f = fold(&thomas.sid, &[thomas.stream(), ryan.stream()], last + 100 * DAY).unwrap();
    assert!(f.is_open(), "auto-close off: open forever");
}

#[test]
fn owner_reopens_after_auto_close() {
    let (mut thomas, ryan) = pair(true);
    thomas.say(T0 + 20 * DAY, Body::Reopened);
    let f = fold(&thomas.sid, &[thomas.stream(), ryan.stream()], T0 + 21 * DAY).unwrap();
    assert!(f.is_open());
    assert!(f.items.iter().any(|i| i.kind == ItemKind::AutoClosed), "the history keeps the auto-close");
}

#[test]
fn editing_expires_after_ten_minutes() {
    let (thomas, mut ryan) = pair(true);
    ryan.say(T0 + 2 * H, Body::WorkingOn { path: "query_builder.py".into() });
    let streams = [thomas.stream(), ryan.stream()];
    assert!(fold(&thomas.sid, &streams, T0 + 2 * H + 9 * 60_000).unwrap().editing.contains_key("query_builder.py"));
    assert!(!fold(&thomas.sid, &streams, T0 + 2 * H + 11 * 60_000).unwrap().editing.contains_key("query_builder.py"));
}

#[test]
fn tasks_are_created_claimed_and_done() {
    let (mut thomas, mut ryan) = pair(true);
    thomas.say(T0 + 2 * H, Body::TaskCreated { id: "t1".into(), title: "recency boost".into() });
    ryan.say(T0 + 3 * H, Body::TaskClaimed { id: "t1".into() });
    ryan.say(T0 + 4 * H, Body::TaskDone { id: "t1".into() });
    let f = fold(&thomas.sid, &[thomas.stream(), ryan.stream()], T0 + 5 * H).unwrap();
    let t = &f.tasks["t1"];
    assert_eq!((t.claimed_by.as_deref(), t.done), (Some("ryan"), true));
}

#[test]
fn a_forked_stream_is_cut_back_to_the_common_prefix() {
    let (thomas, ryan) = pair(true);
    let mut a = Dev::new(2, "ryan", ryan.sid);
    a.blobs = ryan.blobs.clone();
    a.say(T0 + 5 * H, Body::Chat { text: "left".into() });
    let mut b = Dev::new(2, "ryan", ryan.sid);
    b.blobs = ryan.blobs.clone();
    b.say(T0 + 5 * H, Body::Chat { text: "right".into() });
    let f1 = fold(&thomas.sid, &[thomas.stream(), a.stream(), b.stream()], T0 + 6 * H).unwrap();
    let f2 = fold(&thomas.sid, &[b.stream(), thomas.stream(), a.stream()], T0 + 6 * H).unwrap();
    assert_eq!(f1, f2);
    assert_eq!(f1.heads[&ryan.key.tag(&ryan.sid)], 0);
}

// ---------------------------------------------------------------- property

fn arb_body() -> impl Strategy<Value = Body> {
    prop_oneof![
        Just(Body::Joined),
        Just(Body::Closed),
        Just(Body::Reopened),
        "[a-z]{1,8}".prop_map(|text| Body::Chat { text }),
        "[a-z]{1,8}".prop_map(|name| Body::SettingsChanged { patch: SettingsPatch::Name { name } }),
        any::<bool>().prop_map(|on| Body::SettingsChanged { patch: SettingsPatch::AutoClose { on } }),
        (0..3u8).prop_map(|n| Body::TaskCreated { id: format!("t{n}"), title: format!("task {n}") }),
        (0..3u8).prop_map(|n| Body::TaskClaimed { id: format!("t{n}") }),
        (0..3u8).prop_map(|n| Body::TaskDone { id: format!("t{n}") }),
        (0..3u8).prop_map(|n| Body::WorkingOn { path: format!("f{n}.py") }),
    ]
}

/// One to three devices (the owner's, then teammates'), each with a run of
/// events at hour marks, so times often collide across devices.
fn arb_session() -> impl Strategy<Value = (Vec<Vec<(Ms, Body)>>, bool)> {
    (prop::collection::vec(prop::collection::vec(((0..40u64).prop_map(|h| h * H), arb_body()), 0..8), 1..4), any::<bool>())
}

fn build(events: &[Vec<(Ms, Body)>], auto_close: bool) -> (SessionId, Vec<Dev>) {
    let g = genesis(auto_close);
    let sid = g.session_id();
    let logins = ["thomas", "ryan", "priya"];
    let mut devs: Vec<Dev> = Vec::new();
    for (i, evs) in events.iter().enumerate() {
        let mut d = Dev::new(i as u8 + 1, logins[i], sid);
        if i == 0 {
            d.say(T0, Body::Opened { genesis: g.clone() });
        }
        for (dt, body) in evs {
            d.say(T0 + dt, body.clone());
        }
        devs.push(d);
    }
    (sid, devs)
}

proptest! {
    /// Any order, any duplicates, any stale copies: the same Fold.
    #[test]
    fn fold_is_order_independent(
        (events, auto_close) in arb_session(),
        seed in any::<u64>(),
        now_h in 0..400u64,
    ) {
        let (sid, devs) = build(&events, auto_close);
        let now = T0 + now_h * H;
        let full: Vec<Verified> = devs.iter().map(|d| d.stream()).collect();
        let expected = fold(&sid, &full, now);

        // Shuffle, duplicate and add stale prefixes, driven by the seed.
        let mut rng = seed;
        let mut next = || { rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407); rng >> 33 };
        let mut mixed = full.clone();
        for d in &devs {
            if next() % 2 == 0 { mixed.push(d.stream()); }
            if !d.blobs.is_empty() && next() % 2 == 0 {
                let n = (next() as usize) % d.blobs.len();
                mixed.push(d.prefix(n));
            }
        }
        for i in (1..mixed.len()).rev() {
            let j = (next() as usize) % (i + 1);
            mixed.swap(i, j);
        }
        prop_assert_eq!(fold(&sid, &mixed, now), expected);
    }

    /// Folding twice is the same as folding once: sync is idempotent.
    #[test]
    fn fold_is_deterministic((events, auto_close) in arb_session(), now_h in 0..400u64) {
        let (sid, devs) = build(&events, auto_close);
        let streams: Vec<Verified> = devs.iter().map(|d| d.stream()).collect();
        let now = T0 + now_h * H;
        prop_assert_eq!(fold(&sid, &streams, now), fold(&sid, &streams, now));
    }
}

#[test]
fn a_push_noticed_by_two_devices_shows_once() {
    let (mut thomas, mut ryan) = pair(false);
    let push = |to: &str| Body::Pushed { commits: vec![CommitLine { oid: to.into(), subject: "Add a README".into() }], summary: "Add a README".into(), to: Some(to.into()) };
    ryan.say(T0 + 2 * H, push("abc"));
    thomas.say(T0 + 2 * H + 1, push("abc"));
    thomas.say(T0 + 3 * H, push("def"));
    let f = fold(&thomas.sid, &[thomas.stream(), ryan.stream()], T0 + 4 * H).unwrap();
    let pushes: Vec<_> = f.items.iter().filter(|i| matches!(i.kind, ItemKind::Authored(Body::Pushed { .. }))).map(|i| i.login.clone().unwrap()).collect();
    assert_eq!(pushes, vec!["ryan".to_string(), "thomas".to_string()], "one per tip, the first to say it");
}
