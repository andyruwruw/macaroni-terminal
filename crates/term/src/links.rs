//! Finds a plain-text URL or file reference under a cell, for Ctrl/Cmd+click, and decides which
//! URLs may be opened.
//!
//! [`file_at`] finds compiler- and grep-style references (`src/main.rs:12:5`, `C:\x\y.py:3`,
//! MSVC's `y.cpp(12,5)`). It only parses; the app checks the file exists (relative to the
//! terminal's directory) and opens it in a text editor, never with the system's default
//! handler, since output is untrusted.
//!
//! Pure and unit tested. The app calls [`url_at`] on a row of a [`Snapshot`](crate::Snapshot)
//! when the pointer hovers or clicks with the link modifier, and [`is_openable`] before opening
//! anything, including OSC 8 hyperlinks (those come from the emulator instead; see
//! [`Emulator::hyperlink_at`](crate::Emulator::hyperlink_at)).

use crate::grid::{Cell, CellFlags};

/// Schemes macaroni will open. Anything else (`file:`, custom handlers) could launch programs.
pub const OPENABLE_SCHEMES: [&str; 2] = ["https://", "http://"];

/// A URL found in a row of cells.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    /// First column the link covers.
    pub start: usize,
    /// Last column the link covers, inclusive (a wide character's spacer cell isn't counted).
    pub end: usize,
    /// The URL, with trailing sentence punctuation removed.
    pub url: String,
}

/// Whether `url` may be opened: an http(s) URL with something after the scheme. This is the
/// only gate for opening links, because terminal output is untrusted.
pub fn is_openable(url: &str) -> bool {
    OPENABLE_SCHEMES
        .iter()
        .any(|s| url.len() > s.len() && url[..s.len()].eq_ignore_ascii_case(s))
}

/// Characters that end a URL token: whitespace, quotes, and symbols URLs don't contain unescaped.
/// Parentheses and square brackets are allowed here and balanced afterwards.
fn is_delimiter(c: char) -> bool {
    c.is_whitespace() || matches!(c, '"' | '\'' | '<' | '>' | '`' | '|' | '{' | '}' | '^')
}

/// The http(s) URL covering `col` in `row`, if any.
///
/// The token around `col` (between delimiters) is searched for a scheme, so text glued in front
/// of the URL (`(see:https://...`) is skipped. Returns `None` when `col` lands outside the URL
/// part of the token, including on punctuation that was trimmed off its end.
pub fn url_at(row: &[Cell], col: usize) -> Option<Link> {
    // Work on characters, not cells: spacer cells would split a wide character's token. Each
    // entry keeps its real column for the returned span.
    let cells: Vec<(usize, char)> = row
        .iter()
        .enumerate()
        .filter(|(_, c)| !c.flags.contains(CellFlags::WIDE_SPACER))
        .map(|(i, c)| (i, c.ch))
        .collect();
    let at = cells.iter().rposition(|(i, _)| *i <= col)?;
    if is_delimiter(cells[at].1) {
        return None;
    }

    let mut start = at;
    while start > 0 && !is_delimiter(cells[start - 1].1) {
        start -= 1;
    }
    let mut end = at;
    while end + 1 < cells.len() && !is_delimiter(cells[end + 1].1) {
        end += 1;
    }

    let token: String = cells[start..=end].iter().map(|(_, c)| *c).collect();
    // ASCII lowercasing keeps byte offsets, so `offset` is valid in `token` too.
    let lower = token.to_ascii_lowercase();
    let offset = OPENABLE_SCHEMES
        .iter()
        .filter_map(|s| lower.find(s))
        .min()?;
    let mut url: Vec<char> = token[offset..].chars().collect();
    let first = start + token[..offset].chars().count();

    // Trailing punctuation usually belongs to the sentence, not the URL. A closing bracket stays
    // when the URL opened one (Wikipedia-style links).
    while let Some(&last) = url.last() {
        let opens = |open: char| url.iter().filter(|c| **c == open).count();
        let closes = |close: char| url.iter().filter(|c| **c == close).count();
        let strip = match last {
            '.' | ',' | ';' | ':' | '!' | '?' => true,
            ')' => closes(')') > opens('('),
            ']' => closes(']') > opens('['),
            _ => false,
        };
        if !strip {
            break;
        }
        url.pop();
    }

    // `first` and `last` index `cells` (characters); `at` must fall between them.
    let last = first + url.len().checked_sub(1)?;
    if cells[at].0 < cells[first].0 || at > last {
        return None;
    }
    let url: String = url.into_iter().collect();
    is_openable(&url).then(|| Link {
        start: cells[first].0,
        end: cells[last].0,
        url,
    })
}

/// A file reference found in a row of cells: `path:line[:col]`, `path(line[,col])`, or a bare
/// path with a separator and an extension.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileRef {
    /// First column the reference covers.
    pub start: usize,
    /// Last column it covers, inclusive.
    pub end: usize,
    /// The path as written (relative paths are the caller's to resolve).
    pub path: String,
    /// The line, from 1.
    pub line: Option<u32>,
    /// The column, from 1.
    pub column: Option<u32>,
}

/// Characters that end a file reference token: [`is_delimiter`]'s, plus the brackets and
/// punctuation paths sit between in compiler output (`--> src/a.rs:1:2`, `(x.rs:3)`, `x.rs,`).
fn is_file_delimiter(c: char) -> bool {
    is_delimiter(c) || matches!(c, '(' | ')' | '[' | ']' | ',' | ';' | '=')
}

/// The file reference covering `col` in `row`, if any. To keep ordinary words from looking like
/// files, the path needs an extension, and either a line number or a folder separator.
pub fn file_at(row: &[Cell], col: usize) -> Option<FileRef> {
    let cells: Vec<(usize, char)> = row
        .iter()
        .enumerate()
        .filter(|(_, c)| !c.flags.contains(CellFlags::WIDE_SPACER))
        .map(|(i, c)| (i, c.ch))
        .collect();
    let at = cells.iter().rposition(|(i, _)| *i <= col)?;
    if is_file_delimiter(cells[at].1) {
        return None;
    }
    let mut start = at;
    while start > 0 && !is_file_delimiter(cells[start - 1].1) {
        start -= 1;
    }
    let mut end = at;
    while end + 1 < cells.len() && !is_file_delimiter(cells[end + 1].1) {
        end += 1;
    }
    // MSVC's `file(12,5)`: bracketed numbers right after the token, and the index of the `)`.
    let mut paren: Option<(u32, Option<u32>, usize)> = None;
    if cells.get(end + 1).is_some_and(|(_, c)| *c == '(') {
        let rest: Vec<char> = cells[end + 2..].iter().map(|(_, c)| *c).take(24).collect();
        if let Some(close) = rest.iter().position(|c| *c == ')') {
            let inside: String = rest[..close].iter().collect();
            let mut numbers = inside.split(',').map(|n| n.trim().parse::<u32>());
            if let Some(Ok(line)) = numbers.next() {
                let column = numbers.next().and_then(Result::ok);
                paren = Some((line, column, end + 2 + close));
            }
        }
    }

    let token: String = cells[start..=end].iter().map(|(_, c)| *c).collect();
    // Sentence punctuation after the reference isn't part of it.
    let trimmed = token.trim_end_matches(['.', ':', '!', '?', '\'']);
    let mut last = start + trimmed.chars().count().checked_sub(1)?;
    let mut path = trimmed;
    let mut numbers = Vec::new();
    // Up to two `:<digits>` suffixes, from the right: `:line` and `:line:col`. A drive letter's
    // colon (`C:\`) is never followed by only digits, so it stays in the path.
    while numbers.len() < 2
        && let Some((head, tail)) = path.rsplit_once(':')
        && !tail.is_empty()
        && tail.chars().all(|c| c.is_ascii_digit())
    {
        numbers.insert(0, tail.parse::<u32>().ok()?);
        path = head;
    }
    let (mut line, mut column) = (numbers.first().copied(), numbers.get(1).copied());
    if let Some((l, c, close)) = paren
        && line.is_none()
    {
        (line, column) = (Some(l), c);
        last = close;
    }

    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    let has_extension = name.rsplit_once('.').is_some_and(|(stem, ext)| {
        !stem.is_empty()
            && (1..=8).contains(&ext.len())
            && ext.chars().all(|c| c.is_ascii_alphanumeric())
    });
    let has_separator = path.contains(['/', '\\']);
    if !has_extension || !(line.is_some() || has_separator) || path.contains("://") {
        return None;
    }
    if at > last {
        return None;
    }
    Some(FileRef {
        start: cells[start].0,
        end: cells[last].0,
        path: path.to_owned(),
        line,
        column,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(s: &str) -> Vec<Cell> {
        s.chars()
            .map(|ch| Cell {
                ch,
                ..Cell::default()
            })
            .collect()
    }

    #[test]
    fn finds_the_url_under_the_cursor() {
        let r = row("see https://example.com/a?b=1 for more");
        let link = url_at(&r, 10).unwrap();
        assert_eq!(link.url, "https://example.com/a?b=1");
        assert_eq!((link.start, link.end), (4, 28));
        assert_eq!(url_at(&r, 2), None);
        assert_eq!(url_at(&r, 29), None);
    }

    #[test]
    fn trims_sentence_punctuation_but_keeps_balanced_brackets() {
        let r = row("(docs: https://en.wikipedia.org/wiki/Rust_(language)).");
        assert_eq!(
            url_at(&r, 20).unwrap().url,
            "https://en.wikipedia.org/wiki/Rust_(language)"
        );
        let r = row("visit http://x.dev, then");
        assert_eq!(url_at(&r, 8).unwrap().url, "http://x.dev");
        assert_eq!(url_at(&r, 18), None);
    }

    #[test]
    fn only_http_and_https() {
        assert_eq!(url_at(&row("file:///C:/x.exe"), 3), None);
        assert_eq!(url_at(&row("javascript:alert(1)"), 3), None);
        assert!(is_openable("HTTPS://Example.com"));
        assert!(!is_openable("https://"));
        assert!(!is_openable("ms-settings:display"));
    }

    #[test]
    fn finds_file_references() {
        let r = row("  --> crates/app/src/main.rs:12:5");
        let f = file_at(&r, 12).unwrap();
        assert_eq!(f.path, "crates/app/src/main.rs");
        assert_eq!((f.line, f.column), (Some(12), Some(5)));
        assert_eq!((f.start, f.end), (6, 32));

        let f = file_at(&row(r"error at C:\work\x.py:3."), 12).unwrap();
        assert_eq!(
            (f.path.as_str(), f.line, f.column),
            (r"C:\work\x.py", Some(3), None)
        );

        let f = file_at(&row("main.cpp(40,7): error C2065"), 2).unwrap();
        assert_eq!(
            (f.path.as_str(), f.line, f.column),
            ("main.cpp", Some(40), Some(7))
        );
        assert_eq!(f.end, 13);

        let f = file_at(&row("see docs/guide.md for more"), 6).unwrap();
        assert_eq!((f.path.as_str(), f.line), ("docs/guide.md", None));
    }

    #[test]
    fn ordinary_words_and_urls_are_not_files() {
        assert_eq!(file_at(&row("version 1.2 is out"), 9), None);
        assert_eq!(file_at(&row("readme.md alone"), 2), None);
        assert_eq!(file_at(&row("https://a.dev/x.html"), 10), None);
        assert_eq!(file_at(&row("time 12:30"), 7), None);
    }

    #[test]
    fn quotes_and_prefixes_are_not_part_of_the_url() {
        let r = row("url=\"https://a.b/c\"");
        let link = url_at(&r, 10).unwrap();
        assert_eq!(link.url, "https://a.b/c");
        assert_eq!(link.start, 5);
        let r = row("[link](https://a.b/c)");
        assert_eq!(url_at(&r, 10).unwrap().url, "https://a.b/c");
    }
}
