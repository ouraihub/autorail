//! A small unified-diff parser for display: file headers, hunks and numbered lines.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Context,
    Added,
    Removed,
    /// `\ No newline at end of file` and similar notes.
    Note,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub kind: Kind,
    pub old: Option<u32>,
    pub new: Option<u32>,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hunk {
    pub header: String,
    pub lines: Vec<Line>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileDiff {
    /// The new path (`b/…`), or the old one for deletions.
    pub path: String,
    pub old_path: Option<String>,
    pub hunks: Vec<Hunk>,
    pub binary: bool,
}

/// `@@ -a,b +c,d @@` → (a, c).
fn hunk_start(header: &str) -> Option<(u32, u32)> {
    let mut parts = header.strip_prefix("@@ ")?.split(' ');
    let old = parts.next()?.strip_prefix('-')?;
    let new = parts.next()?.strip_prefix('+')?;
    let first = |s: &str| s.split(',').next().and_then(|n| n.parse().ok());
    Some((first(old)?, first(new)?))
}

fn strip_side(path: &str, side: &str) -> Option<String> {
    if path == "/dev/null" {
        return None;
    }
    Some(path.strip_prefix(side).unwrap_or(path).to_owned())
}

pub fn parse(text: &str) -> Vec<FileDiff> {
    let mut files: Vec<FileDiff> = Vec::new();
    let mut lines_numbers = (0u32, 0u32);
    for raw in text.lines() {
        if raw.starts_with("diff --git ") {
            let path = raw.rsplit(" b/").next().unwrap_or_default().to_owned();
            files.push(FileDiff { path, ..FileDiff::default() });
            continue;
        }
        if files.is_empty() {
            files.push(FileDiff::default());
        }
        let Some(file) = files.last_mut() else { continue };
        if file.hunks.is_empty() {
            if let Some(old) = raw.strip_prefix("--- ") {
                file.old_path = strip_side(old.trim_end(), "a/");
                continue;
            }
            if let Some(new) = raw.strip_prefix("+++ ") {
                match strip_side(new.trim_end(), "b/") {
                    Some(path) => file.path = path,
                    None => file.path = file.old_path.clone().unwrap_or_default(),
                }
                continue;
            }
            if let Some(from) = raw.strip_prefix("rename from ") {
                file.old_path = Some(from.to_owned());
                continue;
            }
            if let Some(to) = raw.strip_prefix("rename to ") {
                file.path = to.to_owned();
                continue;
            }
            if raw.starts_with("Binary files ") || raw == "GIT binary patch" {
                file.binary = true;
                continue;
            }
        }
        if raw.starts_with("@@") {
            lines_numbers = hunk_start(raw).unwrap_or((1, 1));
            file.hunks.push(Hunk { header: raw.to_owned(), lines: Vec::new() });
            continue;
        }
        let Some(hunk) = file.hunks.last_mut() else { continue };
        let (old, new) = &mut lines_numbers;
        let line = match raw.as_bytes().first() {
            Some(b'+') => {
                *new += 1;
                Line { kind: Kind::Added, old: None, new: Some(*new - 1), text: raw[1..].to_owned() }
            }
            Some(b'-') => {
                *old += 1;
                Line { kind: Kind::Removed, old: Some(*old - 1), new: None, text: raw[1..].to_owned() }
            }
            Some(b'\\') => Line { kind: Kind::Note, old: None, new: None, text: raw.to_owned() },
            _ => {
                *old += 1;
                *new += 1;
                let text = raw.strip_prefix(' ').unwrap_or(raw).to_owned();
                Line { kind: Kind::Context, old: Some(*old - 1), new: Some(*new - 1), text }
            }
        };
        hunk.lines.push(line);
    }
    files
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIFF: &str = "\
diff --git a/src/a.rs b/src/a.rs
index 1..2 100644
--- a/src/a.rs
+++ b/src/a.rs
@@ -10,3 +10,4 @@ fn main() {
 keep
-old
+new
+more
 tail
\\ No newline at end of file
diff --git a/old.txt b/new.txt
similarity index 90%
rename from old.txt
rename to new.txt
diff --git a/gone.txt b/gone.txt
deleted file mode 100644
--- a/gone.txt
+++ /dev/null
@@ -1 +0,0 @@
-bye
";

    #[test]
    fn hunks_are_numbered_on_both_sides() {
        let files = parse(DIFF);
        assert_eq!(files.len(), 3);
        let lines = &files[0].hunks[0].lines;
        assert_eq!((lines[0].kind, lines[0].old, lines[0].new), (Kind::Context, Some(10), Some(10)));
        assert_eq!((lines[1].kind, lines[1].old, lines[1].new), (Kind::Removed, Some(11), None));
        assert_eq!((lines[2].kind, lines[2].old, lines[2].new), (Kind::Added, None, Some(11)));
        assert_eq!((lines[3].kind, lines[3].new), (Kind::Added, Some(12)));
        assert_eq!((lines[4].kind, lines[4].old, lines[4].new), (Kind::Context, Some(12), Some(13)));
        assert_eq!(lines[5].kind, Kind::Note);
        assert_eq!(files[0].path, "src/a.rs");
    }

    #[test]
    fn renames_and_deletions_keep_meaningful_paths() {
        let files = parse(DIFF);
        assert_eq!((files[1].old_path.as_deref(), files[1].path.as_str()), (Some("old.txt"), "new.txt"));
        assert!(files[1].hunks.is_empty());
        assert_eq!(files[2].path, "gone.txt");
        assert_eq!(files[2].hunks[0].lines[0].kind, Kind::Removed);
    }

    #[test]
    fn binary_and_untracked_no_index_diffs() {
        let files = parse("diff --git a/x.png b/x.png\nBinary files a/x.png and b/x.png differ\n");
        assert!(files[0].binary);
        let untracked = parse("--- /dev/null\n+++ b/n.txt\n@@ -0,0 +1,2 @@\n+a\n+b\n");
        assert_eq!(untracked[0].path, "n.txt");
        assert_eq!(untracked[0].hunks[0].lines[1].new, Some(2));
    }
}
