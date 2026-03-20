use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

/// Structured command for the tmux plugin.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TmuxCommand {
    /// Target tmux session name (e.g., "chops"). None = active session.
    pub session: Option<String>,
    /// Target pane: "shell" (right, :1.2) or "claude" (left, :1.1).
    pub pane: String,
    /// The command text to send via send-keys.
    pub command: String,
}

/// Routing decision returned by parse_intent.
#[derive(Debug, Clone, PartialEq)]
pub enum Intent {
    Vscode(String),
    Termux(String),
    Tmux(TmuxCommand),
}

/// Context for intent parsing — known project names, etc.
pub struct ParseContext {
    pub known_projects: Vec<String>,
}

/// A parsed intent with a confidence score.
#[derive(Debug, Clone)]
pub struct IntentMatch {
    pub intent: Intent,
    pub confidence: f64,
}

// --- Preprocessing ---

const FILLER_WORDS: &[&str] = &[
    "uh", "um", "like", "please", "okay", "ok", "hey", "so", "well", "just", "actually", "right",
];

fn normalize_synonym(word: &str) -> &str {
    match word {
        "execute" | "start" | "launch" | "exec" => "run",
        "message" | "ask" | "send" => "tell",
        "editor" | "code" => "vscode",
        _ => word,
    }
}

/// Strip punctuation, filler words, and normalize synonyms.
fn preprocess(text: &str) -> String {
    text.to_lowercase()
        .replace(['.', ',', '!', '?', ';', ':'], "")
        .split_whitespace()
        .filter(|w| !FILLER_WORDS.contains(w))
        .map(normalize_synonym)
        .collect::<Vec<_>>()
        .join(" ")
}

// --- Regex patterns (compiled once) ---

static RE_TARGETED_TELL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^in\s+(\S+)\s+tell\s+claude\s+(.+)$").unwrap());

static RE_TARGETED_RUN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^in\s+(\S+)\s+run\s+(.+)$").unwrap());

static RE_BARE_RUN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^run\s+(.+)$").unwrap());

static RE_VSCODE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:open\s+)?vscode\s+(.+)$").unwrap());

static RE_TERMUX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:termux|terminal)\s*(.*)$").unwrap());

// --- Fuzzy project matching ---

/// Match a candidate project name against known projects using Jaro-Winkler similarity.
fn match_project(candidate: &str, known: &[String]) -> Option<String> {
    known
        .iter()
        .map(|p| (p.clone(), strsim::jaro_winkler(candidate, p)))
        .filter(|(_, score)| *score > 0.75)
        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
        .map(|(name, _)| name)
}

/// Resolve a project name: exact match first, then fuzzy.
fn resolve_project(candidate: &str, ctx: &ParseContext) -> (String, f64) {
    // Exact match → full confidence.
    if ctx.known_projects.iter().any(|p| p == candidate) {
        return (candidate.to_string(), 1.0);
    }
    // Fuzzy match → reduced confidence.
    if let Some(matched) = match_project(candidate, &ctx.known_projects) {
        return (matched, 0.8);
    }
    // Unknown project — pass through as-is with lower confidence.
    (candidate.to_string(), 0.6)
}

// --- Main entry point ---

/// Parse intent from text. Pure function — no side effects, fully testable.
///
/// Preprocessing normalizes synonyms, strips filler words and punctuation.
/// Regex patterns handle flexible matching. Fuzzy matching corrects project names.
///
/// Supported patterns (after preprocessing):
///   "in <project> run <command>"       → tmux send-keys to project's shell pane
///   "in <project> tell claude <msg>"   → tmux send-keys to project's claude pane
///   "run <command>"                    → tmux send-keys to active session's shell pane
///   "[open] vscode <file>"             → vscode
///   "termux/terminal ..."              → termux
pub fn parse_intent(text: &str, ctx: &ParseContext) -> Option<IntentMatch> {
    let processed = preprocess(text);

    // "in <project> tell claude <message>"
    if let Some(caps) = RE_TARGETED_TELL.captures(&processed) {
        let (project, confidence) = resolve_project(&caps[1], ctx);
        return Some(IntentMatch {
            intent: Intent::Tmux(TmuxCommand {
                session: Some(project),
                pane: "claude".to_string(),
                command: caps[2].to_string(),
            }),
            confidence,
        });
    }

    // "in <project> run <command>"
    if let Some(caps) = RE_TARGETED_RUN.captures(&processed) {
        let (project, confidence) = resolve_project(&caps[1], ctx);
        return Some(IntentMatch {
            intent: Intent::Tmux(TmuxCommand {
                session: Some(project),
                pane: "shell".to_string(),
                command: caps[2].to_string(),
            }),
            confidence,
        });
    }

    // "run <command>" (no project → active session)
    if let Some(caps) = RE_BARE_RUN.captures(&processed) {
        return Some(IntentMatch {
            intent: Intent::Tmux(TmuxCommand {
                session: None,
                pane: "shell".to_string(),
                command: caps[1].to_string(),
            }),
            confidence: 1.0,
        });
    }

    // "[open] vscode <file>"
    if let Some(caps) = RE_VSCODE.captures(&processed) {
        return Some(IntentMatch {
            intent: Intent::Vscode(caps[1].to_string()),
            confidence: 1.0,
        });
    }

    // "termux/terminal ..."
    if RE_TERMUX.is_match(&processed) {
        return Some(IntentMatch {
            intent: Intent::Termux(processed),
            confidence: 1.0,
        });
    }

    None
}

/// Discover projects by scanning a directory for subdirectories containing `.git`.
pub fn discover_projects(base: &std::path::Path) -> Vec<String> {
    let mut projects = Vec::new();
    if let Ok(entries) = std::fs::read_dir(base) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() && path.join(".git").exists() {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    projects.push(name.to_string());
                }
            }
        }
    }
    projects
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_ctx() -> ParseContext {
        ParseContext {
            known_projects: vec![],
        }
    }

    fn test_ctx() -> ParseContext {
        ParseContext {
            known_projects: vec![
                "chops".into(),
                "manta-deploy".into(),
                "atomicguard".into(),
                "dotfiles".into(),
            ],
        }
    }

    // --- Preprocessing ---

    #[test]
    fn preprocess_strips_punctuation() {
        assert_eq!(preprocess("hello, world."), "hello world");
    }

    #[test]
    fn preprocess_removes_filler_words() {
        assert_eq!(preprocess("uh please run cargo test"), "run cargo test");
    }

    #[test]
    fn preprocess_normalizes_synonyms() {
        assert_eq!(
            preprocess("in chops execute cargo test"),
            "in chops run cargo test"
        );
        assert_eq!(
            preprocess("in chops launch cargo test"),
            "in chops run cargo test"
        );
    }

    #[test]
    fn preprocess_handles_combined_noise() {
        assert_eq!(
            preprocess("Okay, please execute cargo test!"),
            "run cargo test"
        );
    }

    // --- Tmux routing: "in <project> run <command>" ---

    #[test]
    fn routes_in_project_run_command() {
        let m = parse_intent("in chops run cargo test", &test_ctx()).unwrap();
        assert_eq!(
            m.intent,
            Intent::Tmux(TmuxCommand {
                session: Some("chops".into()),
                pane: "shell".into(),
                command: "cargo test".into(),
            })
        );
        assert_eq!(m.confidence, 1.0);
    }

    #[test]
    fn routes_in_project_run_case_insensitive() {
        let m = parse_intent("In Chops Run cargo test --release", &test_ctx()).unwrap();
        assert_eq!(
            m.intent,
            Intent::Tmux(TmuxCommand {
                session: Some("chops".into()),
                pane: "shell".into(),
                command: "cargo test --release".into(),
            })
        );
    }

    #[test]
    fn routes_with_synonym_execute() {
        let m = parse_intent("in chops execute cargo test", &test_ctx()).unwrap();
        assert_eq!(
            m.intent,
            Intent::Tmux(TmuxCommand {
                session: Some("chops".into()),
                pane: "shell".into(),
                command: "cargo test".into(),
            })
        );
    }

    #[test]
    fn routes_with_filler_words() {
        let m = parse_intent("okay, please in chops run cargo test.", &test_ctx()).unwrap();
        assert_eq!(
            m.intent,
            Intent::Tmux(TmuxCommand {
                session: Some("chops".into()),
                pane: "shell".into(),
                command: "cargo test".into(),
            })
        );
    }

    #[test]
    fn routes_in_project_tell_claude() {
        let m = parse_intent("in chops tell claude fix the tests", &test_ctx()).unwrap();
        assert_eq!(
            m.intent,
            Intent::Tmux(TmuxCommand {
                session: Some("chops".into()),
                pane: "claude".into(),
                command: "fix the tests".into(),
            })
        );
    }

    #[test]
    fn routes_tell_claude_with_ask_synonym() {
        let m = parse_intent("in chops ask claude fix the tests", &test_ctx()).unwrap();
        assert_eq!(
            m.intent,
            Intent::Tmux(TmuxCommand {
                session: Some("chops".into()),
                pane: "claude".into(),
                command: "fix the tests".into(),
            })
        );
    }

    // --- Fuzzy project matching ---

    #[test]
    fn fuzzy_matches_close_project_name() {
        let m = parse_intent("in chop run cargo test", &test_ctx()).unwrap();
        assert_eq!(
            m.intent,
            Intent::Tmux(TmuxCommand {
                session: Some("chops".into()),
                pane: "shell".into(),
                command: "cargo test".into(),
            })
        );
        assert!(m.confidence < 1.0);
    }

    #[test]
    fn exact_project_match_full_confidence() {
        let m = parse_intent("in chops run ls", &test_ctx()).unwrap();
        assert_eq!(m.confidence, 1.0);
    }

    #[test]
    fn unknown_project_passes_through() {
        let m = parse_intent("in newproject run ls", &test_ctx()).unwrap();
        assert_eq!(
            m.intent,
            Intent::Tmux(TmuxCommand {
                session: Some("newproject".into()),
                pane: "shell".into(),
                command: "ls".into(),
            })
        );
        assert_eq!(m.confidence, 0.6);
    }

    // --- Tmux routing: "run <command>" (active session) ---

    #[test]
    fn routes_run_to_active_session() {
        let m = parse_intent("run cargo build", &empty_ctx()).unwrap();
        assert_eq!(
            m.intent,
            Intent::Tmux(TmuxCommand {
                session: None,
                pane: "shell".into(),
                command: "cargo build".into(),
            })
        );
    }

    #[test]
    fn run_preserves_full_command() {
        let m = parse_intent("run git log --oneline -10", &empty_ctx()).unwrap();
        assert_eq!(
            m.intent,
            Intent::Tmux(TmuxCommand {
                session: None,
                pane: "shell".into(),
                command: "git log --oneline -10".into(),
            })
        );
    }

    // --- Legacy: vscode ---

    #[test]
    fn routes_vscode_open() {
        let m = parse_intent("open vscode README.md", &empty_ctx()).unwrap();
        assert!(matches!(m.intent, Intent::Vscode(_)));
    }

    #[test]
    fn routes_vscode_case_insensitive() {
        let m = parse_intent("Open VSCode main.rs", &empty_ctx()).unwrap();
        assert!(matches!(m.intent, Intent::Vscode(_)));
    }

    #[test]
    fn routes_vscode_with_editor_synonym() {
        let m = parse_intent("open editor main.rs", &empty_ctx()).unwrap();
        assert!(matches!(m.intent, Intent::Vscode(_)));
    }

    // --- Legacy: termux ---

    #[test]
    fn routes_terminal() {
        let m = parse_intent("open terminal", &empty_ctx()).unwrap();
        assert!(matches!(m.intent, Intent::Termux(_)));
    }

    // --- Unhandled ---

    #[test]
    fn unhandled_returns_none() {
        assert!(parse_intent("what time is it", &empty_ctx()).is_none());
        assert!(parse_intent("play music", &empty_ctx()).is_none());
    }

    // --- Realistic whisper.cpp outputs ---

    #[test]
    fn handles_whisper_punctuation_and_filler() {
        let m = parse_intent("Uh, in chops, run cargo test.", &test_ctx()).unwrap();
        assert_eq!(
            m.intent,
            Intent::Tmux(TmuxCommand {
                session: Some("chops".into()),
                pane: "shell".into(),
                command: "cargo test".into(),
            })
        );
    }

    #[test]
    fn handles_please_prefix() {
        let m = parse_intent("Please run ls", &empty_ctx()).unwrap();
        assert_eq!(
            m.intent,
            Intent::Tmux(TmuxCommand {
                session: None,
                pane: "shell".into(),
                command: "ls".into(),
            })
        );
    }
}
