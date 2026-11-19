//! Line diffs for file edits, shown in the permission peek.

use serde::{Deserialize, Serialize};
use std::time::Duration;

use similar::{ChangeTag, TextDiff};

/// Lines kept around each change.
const CONTEXT: usize = 2;
/// Longest a single diff may take. Past it the diff is coarser (bigger changed blocks)
/// but still correct, so the whole analysis stays inside the app's 50 ms budget.
const DIFF_DEADLINE: Duration = Duration::from_millis(15);
/// Lines shown at most; the rest is summed up as "N more lines".
pub const MAX_LINES: usize = 120;

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "camelCase")]
pub enum LineKind {
    Context,
    Removed,
    Added,
    /// Unchanged lines left out between two changes.
    Gap,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DiffLine {
    pub kind: LineKind,
    pub text: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Diff {
    pub path: String,
    /// File extension, lower-case, for syntax colours ("rs", "ts", "py"...).
    pub lang: String,
    pub lines: Vec<DiffLine>,
    pub added: usize,
    pub removed: usize,
    /// More lines changed than are shown.
    pub more: usize,
    /// The text reached Atoll cut short (the bridge keeps 4,000 bytes per string).
    pub cut: bool,
    /// A new file (nothing to compare with).
    pub new_file: bool,
}

fn lang_of(path: &str) -> String {
    let base = path.replace('\\', "/").rsplit('/').next().unwrap_or("").to_lowercase();
    match base.rsplit_once('.') {
        Some((_, ext)) if !ext.is_empty() && ext.len() <= 10 => ext.to_string(),
        _ => base,
    }
}

/// The bridge appends "…" to strings it cut at 4,000 bytes.
pub fn was_cut(s: &str) -> bool {
    s.len() >= 3_990 && s.ends_with('…')
}

impl Diff {
    pub fn new(path: &str) -> Diff {
        Diff { path: path.to_string(), lang: lang_of(path), lines: vec![], added: 0, removed: 0, more: 0, cut: false, new_file: false }
    }

    fn push(&mut self, kind: LineKind, text: &str) {
        match kind {
            LineKind::Added => self.added += 1,
            LineKind::Removed => self.removed += 1,
            _ => {}
        }
        if self.lines.len() >= MAX_LINES {
            if kind != LineKind::Context && kind != LineKind::Gap {
                self.more += 1;
            }
            return;
        }
        // Long lines are clipped: the peek shows a glance, not the file.
        let text: String = if text.chars().count() > 400 { text.chars().take(400).collect::<String>() + "…" } else { text.to_string() };
        self.lines.push(DiffLine { kind, text: text.trim_end_matches(['\n', '\r']).to_string() });
    }

    /// Add the change from `old` to `new` (one Edit, or one edit of a MultiEdit).
    pub fn add_change(&mut self, old: &str, new: &str) {
        if was_cut(old) || was_cut(new) {
            self.cut = true;
        }
        if !self.lines.is_empty() {
            self.push(LineKind::Gap, "");
        }
        let diff = TextDiff::configure().timeout(DIFF_DEADLINE).diff_lines(old, new);
        let groups = diff.grouped_ops(CONTEXT);
        for (gi, group) in groups.iter().enumerate() {
            if gi > 0 {
                self.push(LineKind::Gap, "");
            }
            for op in group {
                for change in diff.iter_changes(op) {
                    let kind = match change.tag() {
                        ChangeTag::Equal => LineKind::Context,
                        ChangeTag::Delete => LineKind::Removed,
                        ChangeTag::Insert => LineKind::Added,
                    };
                    self.push(kind, change.value());
                }
            }
        }
    }

    /// A whole new file: every line is added.
    pub fn add_new_file(&mut self, content: &str) {
        self.new_file = true;
        self.cut |= was_cut(content);
        for line in content.lines() {
            self.push(LineKind::Added, line);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edit_shows_change_with_context() {
        let mut d = Diff::new("src/a.rs");
        d.add_change("a\nb\nc\nd\ne\nf\n", "a\nb\nc\nX\ne\nf\n");
        assert_eq!(d.lang, "rs");
        assert_eq!((d.added, d.removed), (1, 1));
        let kinds: Vec<LineKind> = d.lines.iter().map(|l| l.kind).collect();
        assert_eq!(kinds, [LineKind::Context, LineKind::Context, LineKind::Removed, LineKind::Added, LineKind::Context, LineKind::Context]);
        assert_eq!(d.lines[3].text, "X");
    }

    #[test]
    fn long_changes_are_capped_and_counted() {
        let mut d = Diff::new("x.txt");
        let new: String = (0..500).map(|i| format!("line {i}\n")).collect();
        d.add_new_file(&new);
        assert_eq!(d.added, 500);
        assert_eq!(d.lines.len(), MAX_LINES);
        assert_eq!(d.more, 500 - MAX_LINES);
        assert!(d.new_file);
    }

    #[test]
    fn a_huge_rewrite_stays_fast() {
        let old = "export const x = 1;
".repeat(5_000);
        let new = "export const x = 2;
".repeat(5_000);
        let t = std::time::Instant::now();
        let mut d = Diff::new("big.ts");
        d.add_change(&old, &new);
        assert!(t.elapsed() < std::time::Duration::from_millis(40), "{:?}", t.elapsed());
        assert_eq!((d.added, d.removed), (5_000, 5_000));
    }

    #[test]
    fn notices_text_the_bridge_cut() {
        let mut d = Diff::new("x.txt");
        let long = format!("{}…", "a".repeat(4_000));
        d.add_change("a", &long);
        assert!(d.cut);
    }
}
