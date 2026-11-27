//! The Activity window's timeline (SPEC 2.5, 2.6, 10): the session as entries
//! ready to draw, newest first. Answers are attached to their question instead
//! of standing alone. Pure: a Fold in, data out; the window does the layout.

use crate::github::GitHubRepo;
use crate::ledger::{Body, Fold, ItemKind, Ms, PostKind, SettingsPatch, WhoCanJoin};
use serde::Serialize;
use std::collections::BTreeMap;

/// The most entries one timeline carries.
pub const MAX_ENTRIES: usize = 300;

#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct Timeline {
    pub name: String,
    pub branch: String,
    pub open: bool,
    /// The person reading: their entries read "You" and "Your agent".
    pub me: String,
    /// Everyone in the session, owner first, then in the order they joined
    /// (the window colors avatars by this order).
    pub members: Vec<String>,
    /// Newest first.
    pub entries: Vec<Entry>,
}

/// One line of the timeline: "<who> <verb>", an optional label, quote and link.
#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub at: Ms,
    /// None for what the session itself did (closed after 14 days, closing soon).
    pub login: Option<String>,
    /// Written by the person's agent rather than the person.
    pub agent: bool,
    pub verb: String,
    /// "Question", "Second opinion", "Finding" or "Working on".
    pub label: Option<&'static str>,
    pub quote: Option<String>,
    /// "View changes" on a push.
    pub link: Option<String>,
    /// Answers to a question, oldest first.
    pub answers: Vec<Answer>,
}

#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct Answer {
    pub at: Ms,
    pub login: String,
    pub agent: bool,
    pub text: String,
    /// Given by a read-only copy while the person was away.
    pub background: bool,
}

/// `repo`: the GitHub repo behind the checkout's configured remote (as the
/// share link reads it); without one, the URL the session recorded.
pub fn timeline(f: &Fold, me: &str, repo: Option<GitHubRepo>) -> Timeline {
    let repo = repo.or_else(|| GitHubRepo::parse(&f.genesis.repo));
    let mut answers: BTreeMap<&str, Vec<Answer>> = BTreeMap::new();
    for item in &f.items {
        if let (ItemKind::Authored(Body::Answer { question, text, background }), Some(login)) = (&item.kind, &item.login) {
            answers.entry(question.as_str()).or_default().push(Answer {
                at: item.at,
                login: login.clone(),
                agent: item.agent.is_some(),
                text: text.clone(),
                background: *background,
            });
        }
    }
    let mut entries: Vec<Entry> = f
        .items
        .iter()
        .rev()
        .filter_map(|item| {
            let entry = |verb: String| Entry {
                at: item.at,
                login: item.login.clone(),
                agent: item.agent.is_some(),
                verb,
                label: None,
                quote: None,
                link: None,
                answers: Vec::new(),
            };
            let body = match &item.kind {
                ItemKind::AutoClosed => return Some(entry("The session closed after 14 days without activity.".into())),
                ItemKind::ClosingSoon { .. } => return Some(entry("The session closes in under 2 days unless someone works in it.".into())),
                ItemKind::Authored(body) => body,
            };
            Some(match body {
                Body::Opened { .. } => entry("started the session".into()),
                Body::Closed => entry("closed the session".into()),
                Body::Reopened => entry("reopened the session".into()),
                Body::SettingsChanged { patch } => entry(match patch {
                    SettingsPatch::Name { name } => format!("renamed the session to \"{name}\""),
                    SettingsPatch::AutoClose { on: true } => "turned on auto-close".into(),
                    SettingsPatch::AutoClose { on: false } => "turned off auto-close".into(),
                    SettingsPatch::WhoCanJoin { who: WhoCanJoin::Anyone } => "let anyone with repo access join".into(),
                    SettingsPatch::WhoCanJoin { who: WhoCanJoin::Only { logins } } => format!("let only {} join", logins.iter().cloned().collect::<Vec<_>>().join(", ")),
                }),
                Body::Joined => entry("joined".into()),
                Body::Pushed { commits, .. } => {
                    let n = commits.len();
                    let mut e = entry(format!("pushed {n} commit{}", if n == 1 { "" } else { "s" }));
                    e.quote = Some(commits.iter().map(|c| c.subject.as_str()).collect::<Vec<_>>().join("\n"));
                    e.link = match (&repo, commits.first(), commits.last()) {
                        (Some(r), Some(c), _) if n == 1 => Some(format!("https://github.com/{}/{}/commit/{}", r.owner, r.name, c.oid)),
                        (Some(r), Some(first), Some(last)) => Some(format!("https://github.com/{}/{}/compare/{}%5E...{}", r.owner, r.name, first.oid, last.oid)),
                        _ => None,
                    };
                    e
                }
                Body::BranchRewritten { .. } => entry("rewrote the shared branch (force-push)".into()),
                Body::Conflict { files } => {
                    let mut e = entry("hit a rebase conflict; nothing was pushed".into());
                    e.quote = Some(files.join(", "));
                    e
                }
                Body::WorkingOn { path } => entry(format!("is editing {path}")),
                Body::TurnSummary { text } => {
                    let mut e = entry("finished a turn".into());
                    e.quote = Some(text.clone());
                    e
                }
                Body::Post { about, text } => {
                    let mut e = entry(match about { PostKind::Working => "is working on", PostKind::Finding => "found something" }.into());
                    e.label = Some(match about { PostKind::Working => "Working on", PostKind::Finding => "Finding" });
                    e.quote = Some(text.clone());
                    e
                }
                Body::Ask { id, to, text } => {
                    let mut e = entry(match to.as_deref() {
                        Some(t) if t == me => "asked you".into(),
                        Some(t) => format!("asked {t}"),
                        None => "wants a second opinion".into(),
                    });
                    e.label = Some(if to.is_some() { "Question" } else { "Second opinion" });
                    e.quote = Some(text.clone());
                    e.answers = answers.remove(id.as_str()).unwrap_or_default();
                    e
                }
                Body::Answer { .. } => return None, // shown under its question
                Body::TaskCreated { title, .. } => {
                    let mut e = entry("added a task".into());
                    e.quote = Some(title.clone());
                    e
                }
                Body::TaskClaimed { id } => entry(format!("took the task {}", f.tasks.get(id).map(|t| t.title.as_str()).unwrap_or(id))),
                Body::TaskDone { id } => entry(format!("finished the task {}", f.tasks.get(id).map(|t| t.title.as_str()).unwrap_or(id))),
                Body::Chat { text } | Body::Decision { text } | Body::Question { text, .. } | Body::Handoff { text, .. } => {
                    let mut e = entry("said".into());
                    e.quote = Some(text.clone());
                    e
                }
                Body::Unknown { .. } => return None,
            })
        })
        .collect();
    entries.truncate(MAX_ENTRIES);

    let mut members: Vec<(&String, Ms)> = f.members.iter().map(|(l, m)| (l, if *l == f.genesis.owner { 0 } else { m.joined })).collect();
    members.sort_by_key(|(l, at)| (*at, (*l).clone()));
    Timeline {
        name: f.settings.name.clone(),
        branch: f.genesis.branch.clone(),
        open: f.is_open(),
        me: me.to_string(),
        members: members.into_iter().map(|(l, _)| l.clone()).collect(),
        entries,
    }
}
