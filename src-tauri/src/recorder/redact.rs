//! Secret redaction, applied to every event before it is written.
//! Rules: docs/contracts/event-store.md section 3.
//!
//! Two passes:
//! 1. By key: an object key that names a secret has its whole value replaced.
//! 2. By shape: known token formats and secret-looking assignments inside any
//!    string are replaced, wherever they appear (commands, file contents, prompts).
//!
//! Known limit: a secret with no recognisable name or shape cannot be found.

use std::sync::OnceLock;

use regex::Regex;
use serde_json::Value;

pub const MASK: &str = "[redacted]";

fn secret_key() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(r"(?i)(token|secret|password|passwd|api[_-]?key|authorization|cookie|credential|private[_-]?key|session[_-]?key)").unwrap()
    })
}

/// Identifiers that happen to contain a secret-like word.
fn key_is_exempt(key: &str) -> bool {
    matches!(key, "session_id" | "tool_use_id" | "transcript_path" | "agent_transcript_path")
}

/// Whole-match patterns: the entire match becomes [redacted].
fn token_patterns() -> &'static [Regex] {
    static R: OnceLock<Vec<Regex>> = OnceLock::new();
    R.get_or_init(|| {
        [
            // PEM private keys (multi-line).
            r"(?s)-----BEGIN [A-Z0-9 ]*PRIVATE KEY-----.*?-----END [A-Z0-9 ]*PRIVATE KEY-----",
            // Anthropic / OpenAI style.
            r"\bsk-(?:ant-)?[A-Za-z0-9_\-]{20,}",
            // GitHub.
            r"\b(?:ghp|gho|ghu|ghs|ghr)_[A-Za-z0-9]{20,}",
            r"\bgithub_pat_[A-Za-z0-9_]{20,}",
            // AWS access key IDs.
            r"\b(?:AKIA|ASIA)[A-Z0-9]{16}\b",
            // Slack.
            r"\bxox[abprs]-[A-Za-z0-9\-]{10,}",
            // Google API keys.
            r"\bAIza[0-9A-Za-z_\-]{35}",
            // JWTs.
            r"\beyJ[A-Za-z0-9_\-]{8,}\.[A-Za-z0-9_\-]{8,}\.[A-Za-z0-9_\-]{8,}",
        ]
        .iter()
        .map(|p| Regex::new(p).unwrap())
        .collect()
    })
}

/// Patterns whose capture group 1 is kept and the rest replaced:
/// `<kept>[redacted]`.
fn keep_prefix_patterns() -> &'static [Regex] {
    static R: OnceLock<Vec<Regex>> = OnceLock::new();
    R.get_or_init(|| {
        [
            // Authorization schemes.
            r"(?i)(\b(?:bearer|basic)\s+)[A-Za-z0-9._~+/\-]{8,}=*",
            // Password in a URL: scheme://user:pass@host.
            r"([a-zA-Z][a-zA-Z0-9+.\-]*://[^\s:/@]+:)[^\s@/]+(@)",
            // NAME=value / NAME: value / "name": "value" / $env:NAME = value, with a secret-like name.
            r#"(?i)((?:\$env:)?[A-Za-z0-9_\-]*(?:token|secret|password|passwd|api[_-]?key|private[_-]?key|access[_-]?key|client[_-]?secret)[A-Za-z0-9_\-]*["']?\s*[:=]\s*["']?)[^\s"',;]{4,}"#,
            // --token value / --password=value / -p value is too broad, so only long flags.
            r"(?i)(--(?:token|secret|password|passwd|api-key|apikey|access-key)[= ]\s*)\S{4,}",
        ]
        .iter()
        .map(|p| Regex::new(p).unwrap())
        .collect()
    })
}

/// Redact secrets inside one string. Returns None when nothing changed.
pub fn redact_str(s: &str) -> Option<String> {
    let mut out = std::borrow::Cow::Borrowed(s);
    for re in token_patterns() {
        if re.is_match(&out) {
            out = std::borrow::Cow::Owned(re.replace_all(&out, MASK).into_owned());
        }
    }
    for (i, re) in keep_prefix_patterns().iter().enumerate() {
        if re.is_match(&out) {
            let rep = if i == 1 { format!("${{1}}{MASK}${{2}}") } else { format!("${{1}}{MASK}") };
            out = std::borrow::Cow::Owned(re.replace_all(&out, rep.as_str()).into_owned());
        }
    }
    match out {
        std::borrow::Cow::Borrowed(_) => None,
        std::borrow::Cow::Owned(s) => Some(s),
    }
}

/// Redact a whole JSON value in place (both passes).
pub fn redact_value(v: &mut Value) {
    match v {
        Value::String(s) => {
            if let Some(r) = redact_str(s) {
                *s = r;
            }
        }
        Value::Array(items) => items.iter_mut().for_each(redact_value),
        Value::Object(map) => {
            for (k, val) in map.iter_mut() {
                if !key_is_exempt(k) && secret_key().is_match(k) && !matches!(val, Value::Null | Value::Bool(_)) {
                    // Keep the shape readable: objects/arrays of secrets become one mask.
                    *val = Value::String(MASK.into());
                } else {
                    redact_value(val);
                }
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn r(s: &str) -> String {
        redact_str(s).unwrap_or_else(|| s.to_string())
    }

    #[test]
    fn redacts_known_token_shapes() {
        assert_eq!(r("key sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123 end"), "key [redacted] end");
        assert_eq!(r("export X=ghp_abcdefghijklmnopqrstuvwxyz0123"), "export X=[redacted]");
        assert_eq!(r("AKIAABCDEFGHIJKLMNOP"), "[redacted]");
        assert_eq!(r("xoxb-1234567890-abcdef"), "[redacted]");
        assert_eq!(r("AIzaSyA1234567890abcdefghijklmnopqrstuv"), "[redacted]");
        assert_eq!(r("t=eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.abcdefghijklmnop"), "t=[redacted]");
        let pem = "a\n-----BEGIN RSA PRIVATE KEY-----\nMIIabc\nxyz\n-----END RSA PRIVATE KEY-----\nb";
        assert_eq!(r(pem), "a\n[redacted]\nb");
    }

    #[test]
    fn redacts_assignments_headers_and_urls() {
        assert_eq!(r("curl -H 'Authorization: Bearer abc.def.ghijk123'"), "curl -H 'Authorization: Bearer [redacted]'");
        assert_eq!(r("git clone https://me:hunter22@github.com/x/y"), "git clone https://me:[redacted]@github.com/x/y");
        assert_eq!(r("API_KEY=abcd1234 npm start"), "API_KEY=[redacted] npm start");
        assert_eq!(r(r#"{"password": "letmein!"}"#), r#"{"password": "[redacted]"}"#);
        assert_eq!(r("$env:OPENAI_API_KEY = 'zzzz9999'"), "$env:OPENAI_API_KEY = '[redacted]'");
        assert_eq!(r("tool --token abcdef123"), "tool --token [redacted]");
    }

    #[test]
    fn leaves_ordinary_text_alone() {
        for s in [
            "cargo test --release",
            "Edit src/recorder/redact.rs",
            "the tokenizer splits words",
            "session_id abc-123",
            "https://example.com/path?q=1",
            "skip this",
        ] {
            assert_eq!(redact_str(s), None, "{s}");
        }
    }

    #[test]
    fn redacts_by_key_but_keeps_identifiers() {
        let mut v = json!({
            "session_id": "s1",
            "tool_use_id": "t1",
            "tool_input": { "command": "echo hi", "headers": { "Authorization": "x", "Cookie": ["a"] }, "api_key": "k" },
            "env": { "GITHUB_TOKEN": "plainvalue" , "flag": true, "password": null }
        });
        redact_value(&mut v);
        assert_eq!(v["session_id"], "s1");
        assert_eq!(v["tool_use_id"], "t1");
        assert_eq!(v["tool_input"]["command"], "echo hi");
        assert_eq!(v["tool_input"]["headers"]["Authorization"], MASK);
        assert_eq!(v["tool_input"]["headers"]["Cookie"], MASK);
        assert_eq!(v["tool_input"]["api_key"], MASK);
        assert_eq!(v["env"]["GITHUB_TOKEN"], MASK);
        assert_eq!(v["env"]["flag"], true);
        assert_eq!(v["env"]["password"], Value::Null);
    }
}
