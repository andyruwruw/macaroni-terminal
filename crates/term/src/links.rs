//! Finds a plain-text URL under a cell, for Ctrl/Cmd+click, and decides which URLs may be opened.
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
    fn quotes_and_prefixes_are_not_part_of_the_url() {
        let r = row("url=\"https://a.b/c\"");
        let link = url_at(&r, 10).unwrap();
        assert_eq!(link.url, "https://a.b/c");
        assert_eq!(link.start, 5);
        let r = row("[link](https://a.b/c)");
        assert_eq!(url_at(&r, 10).unwrap().url, "https://a.b/c");
    }
}
