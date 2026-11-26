//! Claude Code boundary: hook stdin parsing, hook output, settings.json merge.
//! Spike (2026-11-26): `additionalContext` reaches the model for SessionStart,
//! UserPromptSubmit and PreToolUse, so every reply is `Silent` or `Context`.
#![allow(dead_code)]

use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HookEvent {
    SessionStart,
    Prompt,
    PreEdit,
    Stop,
    SessionEnd,
}

impl HookEvent {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "start" => Self::SessionStart,
            "prompt" => Self::Prompt,
            "pre-edit" => Self::PreEdit,
            "stop" => Self::Stop,
            "end" => Self::SessionEnd,
            _ => return None,
        })
    }

    /// The `hookEventName` Claude expects back in `hookSpecificOutput`.
    pub fn claude_name(self) -> &'static str {
        match self {
            Self::SessionStart => "SessionStart",
            Self::Prompt => "UserPromptSubmit",
            Self::PreEdit => "PreToolUse",
            Self::Stop => "Stop",
            Self::SessionEnd => "SessionEnd",
        }
    }
}

/// Parsed from the hook's stdin. `agent` is "claude:<session_id>".
pub struct HookInput {
    pub agent: String,
    pub cwd: PathBuf,
    pub term: Option<String>, // $HC_TERMINAL_ID (verified inherited by hooks)
    pub tool_path: Option<PathBuf>,
    pub transcript: Option<PathBuf>,
}

pub enum HookReply {
    Silent,
    Context(String),
}

/// Runs one hook. Returns the stdout line, or None for silent.
/// Scaffold: always silent, so installing the hooks early changes nothing.
pub fn hook(_event: HookEvent, _stdin: &str) -> Option<String> {
    None
}

/// Merge our five entries into ~/.claude/settings.json; idempotent, keeps foreign hooks.
pub fn hooks_install(_settings: &Path, _exe: &Path) -> Result<(), String> {
    todo!("not implemented")
}

/// Remove exactly our entries (matched by command prefix).
pub fn hooks_uninstall(_settings: &Path) -> Result<(), String> {
    todo!("not implemented")
}
