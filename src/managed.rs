//! Shared helpers for editing user files: tora owns only the text between its
//! begin/end markers, shows the change, asks before writing, and replaces files
//! atomically (temporary file → rename) after checking nobody changed them.

use dialoguer::Confirm;
use std::{
    fs,
    io::{self, IsTerminal, Write},
    ops::Range,
    path::{Path, PathBuf},
};

/// Reads a file, treating only a truly absent file as `None`.
pub fn read(path: &Path) -> io::Result<Option<String>> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            // A dangling symlink is not an absent file that we may replace.
            if fs::symlink_metadata(path).is_ok() {
                return Err(io::Error::other(format!(
                    "broken symlink: {}",
                    path.display()
                )));
            }
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

/// The byte range of the managed block from `begin` through `end`, if present.
/// Markers must occupy whole lines (LF or CRLF) and appear exactly once each.
pub fn block_range(text: &str, begin: &str, end: &str) -> io::Result<Option<Range<usize>>> {
    let starts: Vec<_> = text.match_indices(begin).collect();
    let ends: Vec<_> = text.match_indices(end).collect();
    match (starts.as_slice(), ends.as_slice()) {
        ([], []) => Ok(None),
        ([(start, _)], [(stop, _)]) if start < stop => {
            let finish = stop + end.len();
            if (*start != 0 && !text[..*start].ends_with('\n'))
                || !text[..*stop].ends_with('\n')
                || !line_break(&text[*start + begin.len()..])
                || (!text[finish..].is_empty() && !line_break(&text[finish..]))
            {
                return Err(io::Error::other(format!(
                    "invalid tora markers ({begin}); repair the file first"
                )));
            }
            Ok(Some(*start..finish))
        }
        _ => Err(io::Error::other(format!(
            "duplicate or incomplete tora markers ({begin}); repair the file first"
        ))),
    }
}

fn line_break(text: &str) -> bool {
    text.starts_with('\n') || text.starts_with("\r\n")
}

/// The file's line ending: CRLF when its first line ends with CRLF, else LF.
pub fn line_ending(text: &str) -> &'static str {
    match text.find('\n') {
        Some(index) if text[..index].ends_with('\r') => "\r\n",
        _ => "\n",
    }
}

/// Replaces the managed block, or appends it terminated by `eol` when absent.
pub fn replace_block(text: &str, range: Option<Range<usize>>, block: &str, eol: &str) -> String {
    match range {
        Some(range) => format!("{}{block}{}", &text[..range.start], &text[range.end..]),
        None => {
            let separator = if text.is_empty() || text.ends_with('\n') {
                ""
            } else {
                eol
            };
            format!("{text}{separator}{block}{eol}")
        }
    }
}

pub struct Change {
    pub path: PathBuf,
    pub before: Option<String>,
    pub after: String,
}

impl Change {
    pub fn new(path: PathBuf, after: String) -> io::Result<Self> {
        let before = read(&path)?;
        Self::with_before(path, before, after)
    }

    /// Like `new`, for callers that already read `before` from `path`.
    pub fn with_before(path: PathBuf, before: Option<String>, after: String) -> io::Result<Self> {
        // Follow dotfile symlinks without replacing the link itself.
        let path = if before.is_some() {
            fs::canonicalize(path)?
        } else {
            path
        };
        Ok(Self {
            path,
            before,
            after,
        })
    }

    pub fn changed(&self) -> bool {
        self.before.as_deref() != Some(&self.after)
    }
}

/// Asks on the terminal (default: no). Without `yes`, a terminal is required.
pub fn confirm(prompt: &str, yes: bool) -> io::Result<bool> {
    if yes {
        return Ok(true);
    }
    if !(io::stdin().is_terminal() && io::stderr().is_terminal()) {
        return Err(io::Error::other(
            "interactive terminal required to confirm changes; review them and pass --yes",
        ));
    }
    Confirm::new()
        .with_prompt(prompt)
        .default(false)
        .interact()
        .map_err(io::Error::other)
}

fn stage(change: &Change) -> io::Result<tempfile::NamedTempFile> {
    let parent = change
        .path
        .parent()
        .ok_or_else(|| io::Error::other("file has no parent directory"))?;
    fs::create_dir_all(parent)?;
    // New files keep tempfile's private (0600) mode.
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    if change.before.is_some() {
        temporary
            .as_file()
            .set_permissions(fs::metadata(&change.path)?.permissions())?;
    }
    temporary.write_all(change.after.as_bytes())?;
    temporary.as_file().sync_all()?;
    Ok(temporary)
}

/// Writes all changes, or none if a file changed since it was read.
pub fn apply(changes: &[Change]) -> io::Result<()> {
    // Prepare all writes first, so an unwritable destination does not leave
    // one file referring to another that could not be written.
    let staged = changes.iter().map(stage).collect::<io::Result<Vec<_>>>()?;
    for change in changes {
        if read(&change.path)? != change.before {
            return Err(io::Error::other(format!(
                "{} changed while tora was preparing the update; no files changed, retry",
                change.path.display()
            )));
        }
    }
    for (index, (change, temporary)) in changes.iter().zip(staged).enumerate() {
        if let Err(error) = temporary.persist(&change.path) {
            let mut failures = Vec::new();
            for previous in changes[..index].iter().rev() {
                let restored = match &previous.before {
                    Some(before) => stage(&Change {
                        path: previous.path.clone(),
                        before: Some(previous.after.clone()),
                        after: before.clone(),
                    })
                    .and_then(|file| {
                        file.persist(&previous.path)
                            .map(|_| ())
                            .map_err(|e| e.error)
                    }),
                    None => fs::remove_file(&previous.path),
                };
                if let Err(error) = restored {
                    failures.push(format!("{}: {error}", previous.path.display()));
                }
            }
            return Err(io::Error::other(format!(
                "could not update {}: {}; rollback errors: {:?}",
                change.path.display(),
                error.error,
                failures
            )));
        }
    }
    Ok(())
}

/// A line diff (`-` removed, `+` added, two spaces unchanged) with up to
/// `context` unchanged lines around each change and `...` for skipped lines.
pub fn diff(before: &str, after: &str, context: usize) -> String {
    let old: Vec<_> = before.lines().collect();
    let new: Vec<_> = after.lines().collect();
    // Longest common subsequence table; managed blocks are small.
    let mut lcs = vec![vec![0usize; new.len() + 1]; old.len() + 1];
    for i in (0..old.len()).rev() {
        for j in (0..new.len()).rev() {
            lcs[i][j] = if old[i] == new[j] {
                lcs[i + 1][j + 1] + 1
            } else {
                lcs[i + 1][j].max(lcs[i][j + 1])
            };
        }
    }
    let mut lines = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < old.len() || j < new.len() {
        if i < old.len() && j < new.len() && old[i] == new[j] {
            lines.push((' ', old[i]));
            i += 1;
            j += 1;
        } else if j < new.len() && (i == old.len() || lcs[i][j + 1] > lcs[i + 1][j]) {
            lines.push(('+', new[j]));
            j += 1;
        } else {
            lines.push(('-', old[i]));
            i += 1;
        }
    }
    let changed: Vec<_> = lines.iter().map(|(kind, _)| *kind != ' ').collect();
    let near = |index: usize| {
        let start = index.saturating_sub(context);
        let end = (index + context + 1).min(lines.len());
        changed[start..end].iter().any(|c| *c)
    };
    let mut out = String::new();
    let mut skipped = false;
    for (index, (kind, line)) in lines.iter().enumerate() {
        if near(index) {
            if skipped {
                out.push_str("  ...\n");
                skipped = false;
            }
            out.push_str(&format!("{kind} {line}\n"));
        } else {
            skipped = true;
        }
    }
    if skipped {
        out.push_str("  ...\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const BEGIN: &str = "# >>> x >>>";
    const END: &str = "# <<< x <<<";

    #[test]
    fn rejects_damaged_or_duplicate_managed_blocks() {
        for text in [
            BEGIN.to_owned(),
            END.to_owned(),
            format!("{END}\n{BEGIN}"),
            format!("{BEGIN}\n{END}\n{BEGIN}\n{END}"),
            format!("echo '{BEGIN}\n{END}'"),
            format!("{BEGIN}{END}"),
        ] {
            assert!(block_range(&text, BEGIN, END).is_err(), "{text}");
        }
    }

    #[test]
    fn replaces_or_appends_block() {
        let block = format!("{BEGIN}\nnew\n{END}");
        let text = format!("a\n{BEGIN}\nold\n{END}\nb\n");
        let range = block_range(&text, BEGIN, END).unwrap();
        assert_eq!(
            replace_block(&text, range, &block, "\n"),
            format!("a\n{block}\nb\n")
        );
        assert_eq!(
            replace_block("a", None, &block, "\n"),
            format!("a\n{block}\n")
        );
        assert_eq!(replace_block("", None, &block, "\n"), format!("{block}\n"));
        assert_eq!(
            replace_block("a\r\nb", None, &block, "\r\n"),
            format!("a\r\nb\r\n{block}\r\n")
        );
    }

    #[test]
    fn accepts_crlf_markers_and_detects_line_endings() {
        let text = format!("a\r\n{BEGIN}\r\nold\r\n{END}\r\nb\r\n");
        let range = block_range(&text, BEGIN, END).unwrap().unwrap();
        assert_eq!(&text[range], format!("{BEGIN}\r\nold\r\n{END}"));
        assert_eq!(line_ending(&text), "\r\n");
        assert_eq!(line_ending("a\nb\r\n"), "\n");
        assert_eq!(line_ending("a"), "\n");
        assert!(block_range(&format!("{BEGIN}\rx\n{END}"), BEGIN, END).is_err());
    }

    #[test]
    fn diff_marks_changes_with_context() {
        let before = "1\n2\n3\n4\n5\n6\n7\n";
        let after = "1\n2\n3\nfour\n5\n6\n7\n";
        assert_eq!(
            diff(before, after, 1),
            "  ...\n  3\n- 4\n+ four\n  5\n  ...\n"
        );
        assert_eq!(diff("", "a\n", 2), "+ a\n");
        assert_eq!(diff("same\n", "same\n", 2), "  ...\n");
    }
}
