//! Clipboard intelligence (owned by the `search-clipboard` branch).
//!
//! - What a text copy is: link, colour, JSON or code.
//! - Secrets inside it (API keys, tokens, private keys, card numbers,
//!   passwords in assignments or URLs), and a masked copy of the text that is
//!   safe to show, index and store in plain form.
//! - One-click transforms: pretty-print / minify JSON, strip tracking
//!   parameters from a link.
//!
//! Everything here is pure (no I/O) so it can be unit-tested. Text
//! recognition for images lives in `ocr.rs`.

use std::ops::Range;

/// Result of looking at one text copy.
#[derive(Debug, Default, PartialEq)]
pub struct Analysis {
    /// "link" | "colour" | "json" | "code" (at most one today).
    pub types: Vec<String>,
    /// Transforms that make sense for this text ("jsonPretty", "jsonMinify", "stripTracking").
    pub transforms: Vec<String>,
    /// Set when the text holds a secret.
    pub secret: Option<Secret>,
}

#[derive(Debug, PartialEq)]
pub struct Secret {
    /// What was found, e.g. "GitHub token" or "2 secrets".
    pub label: String,
    /// The text with every secret replaced by a mask.
    pub masked: String,
}

pub fn analyse(text: &str, detect_secrets: bool) -> Analysis {
    let mut a = Analysis::default();
    let t = text.trim();
    if is_link(t) {
        a.types.push("link".into());
        if strip_tracking(t).is_some() {
            a.transforms.push("stripTracking".into());
        }
    } else if is_colour(t) {
        a.types.push("colour".into());
    } else if is_json(t) {
        a.types.push("json".into());
        a.transforms.push("jsonPretty".into());
        a.transforms.push("jsonMinify".into());
    } else if is_code(text) {
        a.types.push("code".into());
    }
    if detect_secrets {
        a.secret = find_secret(text);
    }
    a
}

// ---------------------------------------------------------------- types

pub fn is_link(t: &str) -> bool {
    if t.is_empty() || t.contains(char::is_whitespace) {
        return false;
    }
    let l = t.to_ascii_lowercase();
    let rest = l.strip_prefix("https://").or_else(|| l.strip_prefix("http://")).or_else(|| l.strip_prefix("www."));
    rest.is_some_and(|r| r.len() >= 3 && r.split(['/', '?', '#']).next().is_some_and(|host| host.contains('.') || host.starts_with("localhost")))
}

pub fn is_colour(t: &str) -> bool {
    let l = t.to_ascii_lowercase();
    if let Some(h) = l.strip_prefix('#') {
        return matches!(h.len(), 3 | 4 | 6 | 8) && h.chars().all(|c| c.is_ascii_hexdigit());
    }
    for f in ["rgb(", "rgba(", "hsl(", "hsla(", "hwb(", "lab(", "lch(", "oklab(", "oklch("] {
        if let Some(inner) = l.strip_prefix(f).and_then(|r| r.strip_suffix(')')) {
            return inner.chars().any(|c| c.is_ascii_digit()) && inner.chars().all(|c| c.is_ascii_alphanumeric() || " ,./%-+".contains(c));
        }
    }
    false
}

pub fn is_json(t: &str) -> bool {
    let wrapped = (t.starts_with('{') && t.ends_with('}')) || (t.starts_with('[') && t.ends_with(']'));
    wrapped && t.len() >= 2 && serde_json::from_str::<serde_json::Value>(t).is_ok()
}

/// A score from shape (semicolons, braces, indentation), keywords and symbol
/// density. Tuned so prose, lists and shell one-liners stay "text".
pub fn is_code(t: &str) -> bool {
    let lines: Vec<&str> = t.lines().filter(|l| !l.trim().is_empty()).collect();
    let n = lines.len();
    if n == 0 || t.len() < 8 {
        return false;
    }
    // SQL statements read like prose to the scores below.
    let up = t.trim_start().to_ascii_uppercase();
    let sql = [("SELECT ", " FROM "), ("INSERT INTO ", " VALUES"), ("UPDATE ", " SET "), ("DELETE FROM ", ""), ("CREATE TABLE ", "(")];
    if sql.iter().any(|(a, b)| up.starts_with(a) && up.contains(b)) && t.starts_with(|c: char| c.is_ascii_uppercase() || c.is_whitespace()) {
        return true;
    }
    // Markup: a fragment that opens and closes tags.
    let trimmed = t.trim();
    if trimmed.starts_with('<') && trimmed.ends_with('>') && (trimmed.contains("</") || trimmed.contains("/>")) {
        return true;
    }
    let mut score = 0;
    let semi = lines.iter().filter(|l| l.trim_end().ends_with(';')).count();
    let brace = lines
        .iter()
        .filter(|l| {
            let x = l.trim();
            x.ends_with('{') || x.starts_with('}') || x.ends_with("):") || x.ends_with("=>")
        })
        .count();
    let indented = lines.iter().filter(|l| l.starts_with("    ") || l.starts_with('\t') || l.starts_with("  ")).count();
    const KW: [&str; 34] = [
        "fn ", "function ", "def ", "class ", "import ", "#include", "const ", "let ", "var ", "return ", "pub ", "impl ",
        "public ", "private ", "static ", "async ", "await ", "if (", "for (", "while (", "else {", "} else", "=>", "->", "::",
        "console.log", "print(", "</", "/>", "SELECT ", " FROM ", "WHERE ", "#!/", "elif ",
    ];
    let kw = lines.iter().filter(|l| KW.iter().any(|k| l.contains(k))).count();
    let visible: Vec<char> = t.chars().filter(|c| !c.is_whitespace()).collect();
    let symbols = visible.iter().filter(|c| "{}()[];=<>".contains(**c)).count();
    let density = symbols as f64 / visible.len().max(1) as f64;

    if semi * 3 >= n && semi > 0 {
        score += 2;
    }
    score += brace.min(2) as i32;
    if indented >= 2 && n >= 3 {
        score += 1;
    }
    if kw * 10 >= n * 3 {
        score += 2;
    } else if kw > 0 {
        score += 1;
    }
    if density >= 0.15 {
        score += 2;
    } else if density >= 0.07 {
        score += 1;
    }
    if n == 1 { score >= 5 } else { score >= 4 }
}

// ---------------------------------------------------------------- transforms

pub fn transform(text: &str, op: &str) -> Result<String, String> {
    let t = text.trim();
    match op {
        "jsonPretty" | "jsonMinify" => {
            let v: serde_json::Value = serde_json::from_str(t).map_err(|e| format!("That isn't valid JSON: {e}"))?;
            let out = if op == "jsonPretty" { serde_json::to_string_pretty(&v) } else { serde_json::to_string(&v) };
            out.map_err(|e| e.to_string())
        }
        "stripTracking" => Ok(strip_tracking(t).unwrap_or_else(|| t.to_string())),
        _ => Err("Unknown transform.".into()),
    }
}

/// Query keys that only track where a click came from.
const TRACKING: [&str; 22] = [
    "fbclid", "gclid", "dclid", "gbraid", "wbraid", "msclkid", "yclid", "twclid", "ttclid", "li_fat_id", "mc_eid", "mc_cid",
    "igshid", "igsh", "_hsenc", "_hsmi", "mkt_tok", "vero_id", "oly_enc_id", "oly_anon_id", "ref_src", "spm",
];

/// The link without tracking parameters, or `None` if it had none.
pub fn strip_tracking(url: &str) -> Option<String> {
    let (main, fragment) = match url.split_once('#') {
        Some((m, f)) => (m, Some(f)),
        None => (url, None),
    };
    let (base, query) = main.split_once('?')?;
    let host = base.to_ascii_lowercase();
    // `si` and `feature` are tracking on YouTube and Spotify share links only.
    let share_host = ["youtube.com", "youtu.be", "spotify.com", "spotify.link"].iter().any(|h| host.contains(h));
    let mut removed = false;
    let kept: Vec<&str> = query
        .split('&')
        .filter(|p| {
            let key = p.split('=').next().unwrap_or("").to_ascii_lowercase();
            let drop = key.starts_with("utm_") || TRACKING.contains(&key.as_str()) || (share_host && (key == "si" || key == "feature" || key == "pp"));
            removed |= drop;
            !drop && !p.is_empty()
        })
        .collect();
    if !removed {
        return None;
    }
    let mut out = base.to_string();
    if !kept.is_empty() {
        out.push('?');
        out.push_str(&kept.join("&"));
    }
    if let Some(f) = fragment {
        out.push('#');
        out.push_str(f);
    }
    Some(out)
}

// ---------------------------------------------------------------- secrets

struct Prefix {
    prefix: &'static str,
    label: &'static str,
    min_tail: usize,
    /// AWS key IDs are exactly 16 more upper-case characters.
    exact: Option<usize>,
}

const PREFIXES: [Prefix; 22] = [
    Prefix { prefix: "sk-ant-", label: "Anthropic API key", min_tail: 20, exact: None },
    Prefix { prefix: "sk-proj-", label: "OpenAI API key", min_tail: 20, exact: None },
    Prefix { prefix: "sk-", label: "API key", min_tail: 20, exact: None },
    Prefix { prefix: "sk_live_", label: "Stripe key", min_tail: 16, exact: None },
    Prefix { prefix: "rk_live_", label: "Stripe key", min_tail: 16, exact: None },
    Prefix { prefix: "sk_test_", label: "Stripe test key", min_tail: 16, exact: None },
    Prefix { prefix: "github_pat_", label: "GitHub token", min_tail: 30, exact: None },
    Prefix { prefix: "ghp_", label: "GitHub token", min_tail: 30, exact: None },
    Prefix { prefix: "gho_", label: "GitHub token", min_tail: 30, exact: None },
    Prefix { prefix: "ghu_", label: "GitHub token", min_tail: 30, exact: None },
    Prefix { prefix: "ghs_", label: "GitHub token", min_tail: 30, exact: None },
    Prefix { prefix: "ghr_", label: "GitHub token", min_tail: 30, exact: None },
    Prefix { prefix: "glpat-", label: "GitLab token", min_tail: 20, exact: None },
    Prefix { prefix: "AKIA", label: "AWS access key", min_tail: 16, exact: Some(16) },
    Prefix { prefix: "ASIA", label: "AWS access key", min_tail: 16, exact: Some(16) },
    Prefix { prefix: "xoxb-", label: "Slack token", min_tail: 10, exact: None },
    Prefix { prefix: "xoxp-", label: "Slack token", min_tail: 10, exact: None },
    Prefix { prefix: "xoxa-", label: "Slack token", min_tail: 10, exact: None },
    Prefix { prefix: "xoxs-", label: "Slack token", min_tail: 10, exact: None },
    Prefix { prefix: "AIza", label: "Google API key", min_tail: 35, exact: None },
    Prefix { prefix: "npm_", label: "npm token", min_tail: 36, exact: None },
    Prefix { prefix: "hf_", label: "Hugging Face token", min_tail: 30, exact: None },
];

/// Words that name a secret in `name = value` / `"name": "value"` / `--name value`.
const NAMES: [&str; 13] = [
    "password", "passwd", "pwd", "passphrase", "secret", "token", "api_key", "apikey", "api-key", "access_key", "private_key",
    "client_secret", "auth",
];

fn token_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'-' || c == b'_'
}

fn boundary_before(b: &[u8], i: usize) -> bool {
    i == 0 || !b[i - 1].is_ascii_alphanumeric()
}

fn find_all<'a>(hay: &'a str, needle: &'a str) -> impl Iterator<Item = usize> + 'a {
    hay.match_indices(needle).map(|(i, _)| i)
}

/// Byte ranges of every secret found, with a label, sorted and non-overlapping.
pub fn secret_ranges(text: &str) -> Vec<(Range<usize>, &'static str)> {
    let b = text.as_bytes();
    let mut found: Vec<(Range<usize>, &'static str)> = Vec::new();

    // PEM private keys: mask the body between the BEGIN and END lines.
    for start in find_all(text, "-----BEGIN ") {
        let line_end = text[start..].find('\n').map(|i| start + i).unwrap_or(text.len());
        if !text[start..line_end].contains("PRIVATE KEY-----") {
            continue;
        }
        let end = text[line_end..].find("-----END").map(|i| line_end + i).unwrap_or(text.len());
        if end > line_end {
            found.push((line_end..end, "Private key"));
        }
    }

    // Known token prefixes.
    for p in &PREFIXES {
        for start in find_all(text, p.prefix) {
            if !boundary_before(b, start) {
                continue;
            }
            let tail_start = start + p.prefix.len();
            let mut end = tail_start;
            while end < b.len() && token_char(b[end]) {
                end += 1;
            }
            let tail = end - tail_start;
            if let Some(n) = p.exact {
                let ok = tail >= n && b[tail_start..tail_start + n].iter().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit());
                if ok && tail == n {
                    found.push((start..tail_start + n, p.label));
                }
            } else if tail >= p.min_tail {
                found.push((start..end, p.label));
            }
        }
    }

    // JWTs: three base64url parts, the first two starting "eyJ".
    for start in find_all(text, "eyJ") {
        if !boundary_before(b, start) {
            continue;
        }
        let mut end = start;
        while end < b.len() && (token_char(b[end]) || b[end] == b'.') {
            end += 1;
        }
        let parts: Vec<&str> = text[start..end].trim_end_matches('.').split('.').collect();
        if parts.len() == 3 && parts[1].starts_with("eyJ") && parts.iter().all(|p| p.len() >= 10) {
            found.push((start..start + parts.iter().map(|p| p.len()).sum::<usize>() + 2, "Token (JWT)"));
        }
    }

    // Passwords inside URLs: scheme://user:password@host
    for i in find_all(text, "://") {
        let auth_start = i + 3;
        let auth_end = text[auth_start..].find(|c: char| c == '/' || c.is_whitespace()).map(|j| auth_start + j).unwrap_or(text.len());
        let auth = &text[auth_start..auth_end];
        if let (Some(at), Some(colon)) = (auth.rfind('@'), auth.find(':')) {
            if colon < at && at - colon > 1 {
                found.push((auth_start + colon + 1..auth_start + at, "Password in a link"));
            }
        }
    }

    // name = value assignments with a secret-like name.
    let lower = text.to_ascii_lowercase();
    for name in NAMES {
        for start in find_all(&lower, name) {
            if let Some(r) = assigned_value(text, start, name.len()) {
                let label = if name.contains("pass") || name == "pwd" { "Password" } else { "Secret" };
                found.push((r, label));
            }
        }
    }

    // Card numbers (Luhn-valid, typical lengths and grouping).
    found.extend(card_numbers(text).into_iter().map(|r| (r, "Card number")));

    // A single copied word that looks like a random key.
    let t = text.trim();
    if found.is_empty() && looks_random(t) {
        let start = text.find(t).unwrap_or(0);
        found.push((start..start + t.len(), "Possible secret"));
    }

    // Sort, then drop anything overlapping an earlier (or longer) match.
    found.sort_by(|a, b| a.0.start.cmp(&b.0.start).then(b.0.end.cmp(&a.0.end)));
    let mut out: Vec<(Range<usize>, &'static str)> = Vec::new();
    for f in found {
        if out.last().is_some_and(|l| f.0.start < l.0.end) {
            continue;
        }
        out.push(f);
    }
    out
}

/// After a secret-like name at `start`, the value it is set to, if any.
fn assigned_value(text: &str, start: usize, len: usize) -> Option<Range<usize>> {
    let b = text.as_bytes();
    // The name may sit inside a longer identifier (DB_PASSWORD, apiToken): skip to its end.
    let mut i = start + len;
    while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_' || b[i] == b'-') {
        i += 1;
    }
    let flag = start >= 2 && &text[start - 2..start] == "--";
    while i < b.len() && (b[i] == b'"' || b[i] == b'\'') {
        i += 1;
    }
    while i < b.len() && b[i] == b' ' {
        i += 1;
    }
    if i < b.len() && (b[i] == b'=' || b[i] == b':') {
        i += 1;
    } else if !(flag && i > start + len) {
        return None;
    }
    while i < b.len() && (b[i] == b' ' || b[i] == b'\t') {
        i += 1;
    }
    let quote = (i < b.len() && (b[i] == b'"' || b[i] == b'\'')).then(|| b[i]);
    if quote.is_some() {
        i += 1;
    }
    let v_start = i;
    while i < b.len() {
        let c = b[i];
        let stop = match quote {
            Some(q) => c == q || c == b'\n',
            None => c.is_ascii_whitespace() || c == b',' || c == b';' || c == b'"' || c == b'\'',
        };
        if stop {
            break;
        }
        i += 1;
    }
    let value = &text[v_start..i];
    let placeholder = value.len() < 6
        || value.starts_with(['$', '<', '{', '%', '*', '[', '('])
        || value.chars().all(|c| c == value.chars().next().unwrap_or(' '))
        || ["true", "false", "null", "none", "undefined", "required", "string"].contains(&value.to_ascii_lowercase().as_str());
    (!placeholder).then_some(v_start..i)
}

fn luhn(digits: &[u8]) -> bool {
    let mut sum = 0u32;
    for (i, d) in digits.iter().rev().enumerate() {
        let mut v = (*d - b'0') as u32;
        if i % 2 == 1 {
            v *= 2;
            if v > 9 {
                v -= 9;
            }
        }
        sum += v;
    }
    sum % 10 == 0
}

fn card_numbers(text: &str) -> Vec<Range<usize>> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if !b[i].is_ascii_digit() || (i > 0 && (b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'.')) {
            i += 1;
            continue;
        }
        let start = i;
        let mut digits = Vec::new();
        let mut groups = vec![0usize];
        let mut seps = 0;
        while i < b.len() {
            if b[i].is_ascii_digit() {
                digits.push(b[i]);
                *groups.last_mut().unwrap() += 1;
                i += 1;
            } else if (b[i] == b' ' || b[i] == b'-') && i + 1 < b.len() && b[i + 1].is_ascii_digit() && digits.len() < 19 {
                seps += 1;
                groups.push(0);
                i += 1;
            } else {
                break;
            }
        }
        let next_ok = i >= b.len() || !(b[i].is_ascii_alphanumeric() || b[i] == b'.');
        let n = digits.len();
        let shaped = if seps == 0 {
            n == 15 || n == 16
        } else {
            groups == [4, 4, 4, 4] || groups == [4, 6, 5] || groups == [4, 4, 4, 4, 3]
        };
        if next_ok && shaped && matches!(digits[0], b'3'..=b'6') && luhn(&digits) {
            out.push(start..i);
        }
    }
    out
}

fn looks_random(t: &str) -> bool {
    let n = t.chars().count();
    if !(24..=256).contains(&n) || t.contains(char::is_whitespace) || is_link(t) || t.contains('@') || t.contains('\\') {
        return false;
    }
    let (up, low, dig) = (t.chars().any(|c| c.is_ascii_uppercase()), t.chars().any(|c| c.is_ascii_lowercase()), t.chars().any(|c| c.is_ascii_digit()));
    if !(up && low && dig) || t.matches('/').count() > 1 {
        return false;
    }
    entropy(t) >= 4.0
}

/// Shannon entropy in bits per character.
fn entropy(t: &str) -> f64 {
    let mut counts = std::collections::HashMap::new();
    for c in t.chars() {
        *counts.entry(c).or_insert(0usize) += 1;
    }
    let n = t.chars().count() as f64;
    counts.values().map(|&c| {
        let p = c as f64 / n;
        -p * p.log2()
    }).sum()
}

/// "sk-a••••3f9a": a short head and tail so you can tell keys apart.
fn mask_one(s: &str, label: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    if label == "Card number" {
        let last: String = chars.iter().filter(|c| c.is_ascii_digit()).rev().take(4).collect::<Vec<_>>().into_iter().rev().collect();
        return format!("•••• {last}");
    }
    if label == "Private key" {
        return "\n••••••••\n".into();
    }
    if chars.len() >= 16 {
        let head: String = chars[..4].iter().collect();
        let tail: String = chars[chars.len() - 4..].iter().collect();
        format!("{head}••••{tail}")
    } else {
        "••••••".into()
    }
}

pub fn mask(text: &str, ranges: &[(Range<usize>, &'static str)]) -> String {
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    for (r, label) in ranges {
        out.push_str(&text[at..r.start]);
        out.push_str(&mask_one(&text[r.clone()], label));
        at = r.end;
    }
    out.push_str(&text[at..]);
    out
}

pub fn find_secret(text: &str) -> Option<Secret> {
    let ranges = secret_ranges(text);
    if ranges.is_empty() {
        return None;
    }
    let label = if ranges.len() == 1 { ranges[0].1.to_string() } else { format!("{} secrets", ranges.len()) };
    Some(Secret { label, masked: mask(text, &ranges) })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(t: &str) -> Vec<String> {
        analyse(t, true).types
    }

    #[test]
    fn detects_types() {
        assert_eq!(kinds("https://example.com/a?b=1"), ["link"]);
        assert_eq!(kinds("  www.bbc.co.uk  "), ["link"]);
        assert!(kinds("see https://example.com now").is_empty());
        assert_eq!(kinds("#1A2b3C"), ["colour"]);
        assert_eq!(kinds("rgb(12 34 56 / 50%)"), ["colour"]);
        assert_eq!(kinds("hsl(210deg, 40%, 50%)"), ["colour"]);
        assert!(kinds("#hashtag").is_empty());
        assert_eq!(kinds(r#"{"a": [1, 2], "b": null}"#), ["json"]);
        assert!(kinds("{not json}").is_empty());
        assert_eq!(kinds("fn main() {\n    let x = 1;\n    println!(\"{x}\");\n}"), ["code"]);
        assert_eq!(kinds("def f(x):\n    return x * 2\n\nprint(f(3))"), ["code"]);
        assert_eq!(kinds("const add = (a, b) => a + b;\nconsole.log(add(1, 2));"), ["code"]);
        assert_eq!(kinds("SELECT id, name FROM users WHERE age > 30;"), ["code"]);
        assert_eq!(kinds("<div class=\"x\"><span>Hi</span></div>"), ["code"]);
    }

    #[test]
    fn prose_is_not_code() {
        for t in [
            "Let me know when you are free; I think we should meet at 5.",
            "Shopping list:\n- eggs\n- milk (2 litres)\n- bread",
            "npm run app",
            "The class starts at 9 and we return home after lunch.",
            "Dear Sir,\n\nThank you for your letter. I will reply soon.\n\nRegards,\nJia",
        ] {
            assert!(kinds(t).is_empty(), "{t}");
        }
    }

    #[test]
    fn finds_and_masks_secrets() {
        let cases = [
            ("ghp_abcdefghijklmnopqrstuvwxyz0123456789", "GitHub token"),
            ("export ANTHROPIC_API_KEY=sk-ant-api03-abcdefghijklmnopqrstuvwx", "Anthropic API key"),
            ("AKIAIOSFODNN7EXAMPLE", "AWS access key"),
            ("xoxb-123456789012-abcdefABCDEF", "Slack token"),
            ("eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dozjgNryP4J3jVmNHl0w5N_XgL0n3I9PlFUP0THsR8U", "Token (JWT)"),
            ("DB_PASSWORD=hunter2hunter2", "Password"),
            ("\"client_secret\": \"9f8e7d6c5b4a\"", "Secret"),
            ("postgres://admin:s3cretpw@db.local:5432/app", "Password in a link"),
            ("4111 1111 1111 1111", "Card number"),
            ("4111111111111111", "Card number"),
            ("q8Zt3LmP0xVb7NcR2wYs5KdJ9hGf", "Possible secret"),
        ];
        for (text, label) in cases {
            let s = find_secret(text).unwrap_or_else(|| panic!("missed: {text}"));
            assert_eq!(s.label, label, "{text}");
            assert!(s.masked.contains('•'), "{text}");
        }
        let pem = "-----BEGIN RSA PRIVATE KEY-----\nMIIEpAIBAAKCAQEA\nabcdef\n-----END RSA PRIVATE KEY-----";
        let s = find_secret(pem).unwrap();
        assert_eq!(s.label, "Private key");
        assert!(!s.masked.contains("MIIEpA"));
        let s = find_secret("4111 1111 1111 1111").unwrap();
        assert_eq!(s.masked, "•••• 1111");
        let s = find_secret("token: ghp_abcdefghijklmnopqrstuvwxyz0123456789").unwrap();
        assert!(!s.masked.contains("klmnop"));
    }

    #[test]
    fn ordinary_text_has_no_secrets() {
        for t in [
            "Hello there, the password is in the usual place.",
            "password: required",
            "API_KEY=${API_KEY}",
            "c9b1e8f0-5a4d-4f2e-9b3a-7d6e5f4c3b2a",
            "3f786850e387550fdab836ed7e6dc881de23001b",
            "1731234567890",
            "Order 1234 5678 9012 3456 shipped",
            "https://www.youtube.com/watch?v=dQw4w9WgXcQ",
            "C:\\Users\\chuah\\OneDrive\\Desktop\\twocodeclone",
            "TheQuickBrownFoxJumpsOverTheLazyDog",
        ] {
            assert!(find_secret(t).is_none(), "false positive: {t} -> {:?}", find_secret(t));
        }
    }

    #[test]
    fn transforms() {
        assert_eq!(transform(" {\"a\":1,\"b\":[1,2]} ", "jsonPretty").unwrap(), "{\n  \"a\": 1,\n  \"b\": [\n    1,\n    2\n  ]\n}");
        assert_eq!(transform("{ \"a\" : 1 }", "jsonMinify").unwrap(), "{\"a\":1}");
        assert_eq!(
            strip_tracking("https://shop.com/p?id=7&utm_source=x&utm_medium=y&fbclid=abc#top").as_deref(),
            Some("https://shop.com/p?id=7#top")
        );
        assert_eq!(strip_tracking("https://youtu.be/abc?si=XYZ").as_deref(), Some("https://youtu.be/abc"));
        assert_eq!(strip_tracking("https://example.com/?si=keep"), None);
        assert_eq!(strip_tracking("https://example.com/a"), None);
        let a = analyse("https://news.site/x?utm_campaign=z", true);
        assert_eq!(a.transforms, ["stripTracking"]);
    }
}
