use regex::Regex;
use std::sync::LazyLock;

pub use chops_common::{Pane, TmuxCommand};

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

/// Action keywords that map to canonical forms.
/// Only applied to the first few structural words, not to command payloads.
fn normalize_synonym(word: &str) -> &str {
    match word {
        "execute" | "start" | "launch" | "exec" => "run",
        "message" | "ask" | "send" => "tell",
        "editor" | "code" => "vscode",
        _ => word,
    }
}

/// Strip filler words, normalize synonyms on structural keywords only.
/// Punctuation is stripped only at word boundaries (trailing/leading), preserving
/// characters within words (e.g., "file.txt" stays intact).
fn preprocess(text: &str) -> String {
    let words: Vec<String> = text
        .to_lowercase()
        .split_whitespace()
        .map(|w| {
            w.trim_matches(|c: char| matches!(c, '.' | ',' | '!' | '?' | ';' | ':'))
                .to_string()
        })
        .filter(|w| !w.is_empty() && !FILLER_WORDS.contains(&w.as_str()))
        .collect();

    // Find where the command payload starts (after structural keywords).
    // Normalize synonyms only for the structural prefix (first 4-5 words).
    let structural_len = detect_structural_prefix(&words);

    words
        .iter()
        .enumerate()
        .map(|(i, w)| {
            if i < structural_len {
                normalize_synonym(w).to_string()
            } else {
                w.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Determine how many leading words are structural keywords vs. command payload.
/// Returns the index where the command payload begins.
fn detect_structural_prefix(words: &[String]) -> usize {
    if words.is_empty() {
        return 0;
    }

    // "in <project> tell claude <payload>" → structural = 4
    if words.len() >= 4
        && words[0] == "in"
        && (words[2] == "tell"
            || normalize_synonym(&words[2]) == "tell"
            || words[2] == "run"
            || normalize_synonym(&words[2]) == "run")
    {
        if (words[2] == "tell" || normalize_synonym(&words[2]) == "tell")
            && words.len() >= 5
            && words[3] == "claude"
        {
            return 4; // "in X tell claude" — payload starts at 4
        }
        return 3; // "in X run" — payload starts at 3
    }

    // "run <payload>" → structural = 1
    if words[0] == "run" || normalize_synonym(&words[0]) == "run" {
        return 1;
    }

    // "open vscode/editor <payload>" → structural = 2
    if words.len() >= 2
        && words[0] == "open"
        && (words[1] == "vscode"
            || normalize_synonym(&words[1]) == "vscode"
            || words[1] == "terminal"
            || words[1] == "termux")
    {
        return 2;
    }

    // "vscode <payload>" → structural = 1
    if words[0] == "vscode" || normalize_synonym(&words[0]) == "vscode" {
        return 1;
    }

    // "terminal/termux ..." → structural = 1
    if words[0] == "terminal" || words[0] == "termux" {
        return 1;
    }

    // Unknown — normalize everything (safe default for non-matching input)
    words.len()
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

const FUZZY_MATCH_THRESHOLD: f64 = 0.75;
const FUZZY_MATCH_CONFIDENCE: f64 = 0.8;
const UNKNOWN_PROJECT_CONFIDENCE: f64 = 0.6;

/// Match a candidate project name against known projects using Jaro-Winkler similarity.
fn match_project(candidate: &str, known: &[String]) -> Option<String> {
    known
        .iter()
        .map(|p| (p.clone(), strsim::jaro_winkler(candidate, p)))
        .filter(|(_, score)| *score > FUZZY_MATCH_THRESHOLD)
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
        return (matched, FUZZY_MATCH_CONFIDENCE);
    }
    // Unknown project — pass through as-is with lower confidence.
    (candidate.to_string(), UNKNOWN_PROJECT_CONFIDENCE)
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
                pane: Pane::Claude,
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
                pane: Pane::Shell,
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
                pane: Pane::Shell,
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

/// Terminator words that signal the end of a multi-segment utterance.
/// Inspired by radio protocol ("over", "over and out").
const TERMINATOR_WORDS: &[&str] = &["over", "out", "done", "end", "finish", "send it"];

/// Check if text ends with a terminator keyword. Returns the text with the
/// terminator stripped if found, or None if no terminator is present.
pub fn strip_terminator(text: &str) -> Option<String> {
    let lower = text.to_lowercase();
    let trimmed = lower.trim_end_matches(|c: char| c.is_ascii_punctuation() || c.is_whitespace());

    for &term in TERMINATOR_WORDS {
        if let Some(prefix) = trimmed.strip_suffix(term) {
            let result = prefix
                .trim_end_matches(|c: char| c.is_ascii_punctuation() || c.is_whitespace())
                .to_string();
            if result.is_empty() {
                return Some(String::new());
            }
            return Some(result);
        }
    }
    None
}

/// Check if text contains a terminator keyword anywhere (for raw transcription checks).
pub fn has_terminator(text: &str) -> bool {
    let lower = text.to_lowercase();
    let trimmed = lower.trim_end_matches(|c: char| c.is_ascii_punctuation() || c.is_whitespace());
    TERMINATOR_WORDS.iter().any(|&term| trimmed.ends_with(term))
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
                pane: Pane::Shell,
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
                pane: Pane::Shell,
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
                pane: Pane::Shell,
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
                pane: Pane::Shell,
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
                pane: Pane::Claude,
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
                pane: Pane::Claude,
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
                pane: Pane::Shell,
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
                pane: Pane::Shell,
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
                pane: Pane::Shell,
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
                pane: Pane::Shell,
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
                pane: Pane::Shell,
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
                pane: Pane::Shell,
                command: "ls".into(),
            })
        );
    }

    // --- Whisper noise corpus ---
    // Realistic whisper.cpp transcription outputs with common artifacts:
    // trailing periods, capitalized sentences, filler words, hesitations,
    // punctuation mid-sentence, synonym substitutions, and project name typos.

    #[test]
    fn whisper_trailing_period() {
        let m = parse_intent("Run cargo test.", &empty_ctx()).unwrap();
        assert_eq!(
            m.intent,
            Intent::Tmux(TmuxCommand {
                session: None,
                pane: Pane::Shell,
                command: "cargo test".into(),
            })
        );
    }

    #[test]
    fn whisper_multiple_filler_words() {
        let m = parse_intent("Um, okay, so, run cargo build.", &empty_ctx()).unwrap();
        assert_eq!(
            m.intent,
            Intent::Tmux(TmuxCommand {
                session: None,
                pane: Pane::Shell,
                command: "cargo build".into(),
            })
        );
    }

    #[test]
    fn whisper_capitalized_sentence() {
        let m = parse_intent("In Chops Tell Claude please review the code", &test_ctx()).unwrap();
        assert_eq!(
            m.intent,
            Intent::Tmux(TmuxCommand {
                session: Some("chops".into()),
                pane: Pane::Claude,
                command: "review the code".into(),
            })
        );
    }

    #[test]
    fn whisper_exclamation_mark() {
        let m = parse_intent("Run cargo test!", &empty_ctx()).unwrap();
        assert_eq!(
            m.intent,
            Intent::Tmux(TmuxCommand {
                session: None,
                pane: Pane::Shell,
                command: "cargo test".into(),
            })
        );
    }

    #[test]
    fn whisper_question_phrasing() {
        // whisper sometimes adds question marks to commands
        let m = parse_intent("In chops run cargo clippy?", &test_ctx()).unwrap();
        assert_eq!(
            m.intent,
            Intent::Tmux(TmuxCommand {
                session: Some("chops".into()),
                pane: Pane::Shell,
                command: "cargo clippy".into(),
            })
        );
    }

    #[test]
    fn whisper_hey_prefix() {
        let m = parse_intent("Hey, run git status.", &empty_ctx()).unwrap();
        assert_eq!(
            m.intent,
            Intent::Tmux(TmuxCommand {
                session: None,
                pane: Pane::Shell,
                command: "git status".into(),
            })
        );
    }

    #[test]
    fn whisper_launch_synonym() {
        let m = parse_intent("In chops launch cargo build --release.", &test_ctx()).unwrap();
        assert_eq!(
            m.intent,
            Intent::Tmux(TmuxCommand {
                session: Some("chops".into()),
                pane: Pane::Shell,
                command: "cargo build --release".into(),
            })
        );
    }

    #[test]
    fn whisper_send_synonym_for_tell() {
        let m = parse_intent("In chops send claude add unit tests", &test_ctx()).unwrap();
        assert_eq!(
            m.intent,
            Intent::Tmux(TmuxCommand {
                session: Some("chops".into()),
                pane: Pane::Claude,
                command: "add unit tests".into(),
            })
        );
    }

    #[test]
    fn whisper_open_editor_synonym() {
        let m = parse_intent("Open editor src/main.rs", &empty_ctx()).unwrap();
        assert!(matches!(m.intent, Intent::Vscode(ref f) if f == "src/main.rs"));
    }

    #[test]
    fn whisper_complex_command_with_pipes() {
        let m = parse_intent("run cat file.txt | grep error", &empty_ctx()).unwrap();
        assert_eq!(
            m.intent,
            Intent::Tmux(TmuxCommand {
                session: None,
                pane: Pane::Shell,
                command: "cat file.txt | grep error".into(),
            })
        );
    }

    #[test]
    fn whisper_complex_command_with_flags() {
        let m = parse_intent("in chops run docker compose up -d --build", &test_ctx()).unwrap();
        assert_eq!(
            m.intent,
            Intent::Tmux(TmuxCommand {
                session: Some("chops".into()),
                pane: Pane::Shell,
                command: "docker compose up -d --build".into(),
            })
        );
    }

    #[test]
    fn whisper_all_noise_combined() {
        // Filler + punctuation + synonym + project typo
        let m = parse_intent("Uh, okay, please in chop execute cargo test.", &test_ctx()).unwrap();
        assert_eq!(
            m.intent,
            Intent::Tmux(TmuxCommand {
                session: Some("chops".into()),
                pane: Pane::Shell,
                command: "cargo test".into(),
            })
        );
        assert!(m.confidence < 1.0); // fuzzy project match
    }

    #[test]
    fn whisper_semicolons_and_colons() {
        let m = parse_intent("Well; in chops: run make build.", &test_ctx()).unwrap();
        assert_eq!(
            m.intent,
            Intent::Tmux(TmuxCommand {
                session: Some("chops".into()),
                pane: Pane::Shell,
                command: "make build".into(),
            })
        );
    }

    #[test]
    fn whisper_bare_run_only_rejected() {
        // "run" alone with no command should not match
        assert!(parse_intent("run", &empty_ctx()).is_none());
    }

    #[test]
    fn whisper_just_filler_rejected() {
        assert!(parse_intent("um, uh, okay.", &empty_ctx()).is_none());
    }

    #[test]
    fn whisper_gibberish_rejected() {
        assert!(
            parse_intent("the quick brown fox jumps over the lazy dog", &empty_ctx()).is_none()
        );
    }

    // --- Fuzzy matching edge cases ---

    #[test]
    fn fuzzy_single_char_deletion() {
        // "chop" → "chops" (missing trailing 's')
        let m = parse_intent("in chop run ls", &test_ctx()).unwrap();
        assert_eq!(
            m.intent,
            Intent::Tmux(TmuxCommand {
                session: Some("chops".into()),
                pane: Pane::Shell,
                command: "ls".into(),
            })
        );
        assert_eq!(m.confidence, 0.8);
    }

    #[test]
    fn fuzzy_single_char_substitution() {
        // "shops" → "chops" (substitution of first char)
        let m = parse_intent("in shops run ls", &test_ctx()).unwrap();
        assert_eq!(
            m.intent,
            Intent::Tmux(TmuxCommand {
                session: Some("chops".into()),
                pane: Pane::Shell,
                command: "ls".into(),
            })
        );
        assert_eq!(m.confidence, 0.8);
    }

    #[test]
    fn fuzzy_too_distant_no_match() {
        // "xyz" is too far from any known project — should pass through at 0.6
        let m = parse_intent("in xyz run ls", &test_ctx()).unwrap();
        assert_eq!(
            m.intent,
            Intent::Tmux(TmuxCommand {
                session: Some("xyz".into()),
                pane: Pane::Shell,
                command: "ls".into(),
            })
        );
        assert_eq!(m.confidence, 0.6);
    }

    #[test]
    fn fuzzy_matches_longer_project_name() {
        // "manta" → "manta-deploy"
        let m = parse_intent("in manta-deplo run ls", &test_ctx()).unwrap();
        assert_eq!(
            m.intent,
            Intent::Tmux(TmuxCommand {
                session: Some("manta-deploy".into()),
                pane: Pane::Shell,
                command: "ls".into(),
            })
        );
        assert_eq!(m.confidence, 0.8);
    }

    #[test]
    fn fuzzy_exact_match_preferred_over_fuzzy() {
        // When exact match exists, should get 1.0 confidence
        let m = parse_intent("in dotfiles run ls", &test_ctx()).unwrap();
        assert_eq!(
            m.intent,
            Intent::Tmux(TmuxCommand {
                session: Some("dotfiles".into()),
                pane: Pane::Shell,
                command: "ls".into(),
            })
        );
        assert_eq!(m.confidence, 1.0);
    }

    // --- Terminator detection ---

    #[test]
    fn strip_terminator_over() {
        assert_eq!(
            strip_terminator("fix the tests over"),
            Some("fix the tests".into())
        );
    }

    #[test]
    fn strip_terminator_over_with_period() {
        assert_eq!(
            strip_terminator("fix the tests. Over."),
            Some("fix the tests".into())
        );
    }

    #[test]
    fn strip_terminator_done() {
        assert_eq!(
            strip_terminator("add error handling done"),
            Some("add error handling".into())
        );
    }

    #[test]
    fn strip_terminator_no_terminator() {
        assert_eq!(strip_terminator("fix the tests"), None);
    }

    #[test]
    fn strip_terminator_only_terminator() {
        assert_eq!(strip_terminator("over"), Some(String::new()));
        assert_eq!(strip_terminator("Over."), Some(String::new()));
    }

    #[test]
    fn has_terminator_works() {
        assert!(has_terminator("over"));
        assert!(has_terminator("Over."));
        assert!(has_terminator("some text done."));
        assert!(!has_terminator("fix the tests"));
        assert!(!has_terminator("run cargo test"));
    }

    #[test]
    fn strip_terminator_send_it() {
        assert_eq!(
            strip_terminator("review the code send it"),
            Some("review the code".into())
        );
    }

    #[test]
    fn fuzzy_does_not_match_completely_different() {
        // "banana" should not match any project
        let m = parse_intent("in banana run ls", &test_ctx()).unwrap();
        assert_eq!(
            m.intent,
            Intent::Tmux(TmuxCommand {
                session: Some("banana".into()),
                pane: Pane::Shell,
                command: "ls".into(),
            })
        );
        assert_eq!(m.confidence, 0.6); // unknown project passthrough
    }
}
