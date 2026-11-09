//! Reading a file for a file tab: what kind it is (by its extension), its text or bytes, and
//! the other files in its folder ([`siblings`]).
//!
//! Everything here blocks and runs off the UI thread (`vscode::off_thread`). [`stamp`] is the
//! cheap check the tab's tick makes; [`read`] runs only when the stamp changed.

use std::path::Path;
use std::time::SystemTime;

/// The most text shown; the rest of a longer file is left out (and the tab says so).
pub const MAX_TEXT: usize = 4 * 1024 * 1024;

/// The largest image shown.
const MAX_IMAGE: u64 = 64 * 1024 * 1024;

/// How much of a file is looked at to tell text from binary.
const SNIFF: usize = 8 * 1024;

/// How a file is shown, by its extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Text, highlighted by its extension when it's code.
    Text,
    /// Markdown: rendered, or its source.
    Markdown,
    /// A still picture iced decodes (PNG, JPEG, WebP, BMP, ICO, TIFF and so on).
    Image,
    /// A video, a sound, or a picture that moves or that iced can't decode: played by the file
    /// tab's page, straight from the file.
    Media(Media),
    /// An SVG drawing.
    Svg,
    /// A diagram in a viewer extension's language (`.mmd`, `.dot`, `.puml`, `.d2`): drawn in
    /// the file tab's page, or its source.
    Diagram(crate::viewers::Extension),
}

impl Kind {
    /// The kind of `path`, by its extension (ignoring case); anything unknown is text.
    pub fn of(path: &Path) -> Self {
        let extension = path
            .extension()
            .map(|e| e.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        match extension.as_str() {
            "md" | "markdown" | "mdown" | "mkd" | "mdx" => Self::Markdown,
            "png" | "jpg" | "jpeg" | "webp" | "bmp" | "ico" | "tif" | "tiff" | "tga" | "pnm"
            | "qoi" | "exr" | "hdr" => Self::Image,
            "gif" | "apng" | "avif" | "heic" | "heif" | "jxl" => Self::Media(Media::Picture),
            "mp4" | "m4v" | "webm" | "mov" | "ogv" | "mkv" => Self::Media(Media::Video),
            "mp3" | "wav" | "ogg" | "oga" | "opus" | "flac" | "m4a" | "aac" | "weba" | "aif"
            | "aiff" => Self::Media(Media::Audio),
            "svg" => Self::Svg,
            other => crate::viewers::Extension::for_file(other).map_or(Self::Text, Self::Diagram),
        }
    }
}

/// What kind of media a [`Kind::Media`] file is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Media {
    /// A picture: animated (GIF, APNG) or in a format only the web view decodes (AVIF, HEIC).
    Picture,
    /// A video, with the web view's player.
    Video,
    /// A sound, with the web view's player.
    Audio,
}

/// What a file looked like when it was read: if either part changes, it's read again.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Stamp {
    /// When it was last written.
    modified: Option<SystemTime>,
    /// Its size in bytes.
    len: u64,
}

impl Stamp {
    /// The file's size in bytes.
    pub fn bytes(self) -> u64 {
        self.len
    }

    /// A short tag that changes with the stamp, to make a page load the file again.
    pub fn tag(self) -> String {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::hash::DefaultHasher::new();
        self.hash(&mut hasher);
        format!("{:x}", hasher.finish())
    }
}

/// What was read.
#[derive(Debug, Clone)]
pub enum Read {
    /// Text (for [`Kind::Text`], [`Kind::Markdown`] and [`Kind::Diagram`]), with Windows line
    /// endings made `\n`.
    Text {
        /// At most [`MAX_TEXT`] of it.
        text: String,
        /// The file is longer than what's in `text`.
        cut: bool,
    },
    /// An image's or SVG's bytes, decoded when drawn.
    Bytes(Vec<u8>),
    /// Media, which the page loads from the file itself: nothing is read here.
    Media,
    /// A file that isn't text (it has NUL bytes), or an image too large to show.
    Binary,
    /// A folder, not a file.
    Folder,
    /// Nothing at the path.
    Missing,
    /// Reading failed.
    Failed(String),
}

/// `path`'s stamp, `None` when there's nothing there.
pub fn stamp(path: &Path) -> Option<Stamp> {
    let meta = std::fs::metadata(path).ok()?;
    Some(Stamp {
        modified: meta.modified().ok(),
        len: meta.len(),
    })
}

/// Reads `path` as `kind`, with its stamp from just before.
pub fn read(path: &Path, kind: Kind) -> (Option<Stamp>, Read) {
    let stamp = stamp(path);
    let read = match std::fs::metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Read::Missing,
        Err(e) => Read::Failed(e.to_string()),
        Ok(meta) if meta.is_dir() => Read::Folder,
        Ok(meta) => match kind {
            Kind::Image | Kind::Svg if meta.len() > MAX_IMAGE => Read::Binary,
            Kind::Image | Kind::Svg => match std::fs::read(path) {
                Ok(bytes) => Read::Bytes(bytes),
                Err(e) => Read::Failed(e.to_string()),
            },
            Kind::Text | Kind::Markdown | Kind::Diagram(_) => read_text(path),
            Kind::Media(_) => Read::Media,
        },
    };
    (stamp, read)
}

/// The start of a file as text, at most [`MAX_TEXT`] of it.
fn read_text(path: &Path) -> Read {
    use std::io::Read as _;
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(e) => return Read::Failed(e.to_string()),
    };
    let mut bytes = Vec::new();
    // One byte past the limit says whether there's more.
    if let Err(e) = file.take(MAX_TEXT as u64 + 1).read_to_end(&mut bytes) {
        return Read::Failed(e.to_string());
    }
    let cut = bytes.len() > MAX_TEXT;
    bytes.truncate(MAX_TEXT);
    match decode(&bytes) {
        Some(text) => Read::Text {
            text: text.replace("\r\n", "\n"),
            cut,
        },
        None => Read::Binary,
    }
}

/// `bytes` as text: UTF-16 with a byte order mark (what Windows PowerShell writes), else UTF-8
/// (without its mark), with anything invalid replaced. `None` for binary: NUL bytes near the
/// start, which text other than UTF-16 doesn't have.
fn decode(bytes: &[u8]) -> Option<String> {
    let utf16 = |rest: &[u8], from: fn([u8; 2]) -> u16| {
        let units: Vec<u16> = rest
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| from(*pair))
            .collect();
        String::from_utf16_lossy(&units)
    };
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        return Some(utf16(rest, u16::from_le_bytes));
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        return Some(utf16(rest, u16::from_be_bytes));
    }
    if bytes[..bytes.len().min(SNIFF)].contains(&0) {
        return None;
    }
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    // A cut can split the last character; lossy decoding turns it into one replacement mark.
    Some(String::from_utf8_lossy(bytes).into_owned())
}

/// The files in `path`'s folder (not folders), `path` among them when it exists, sorted by
/// name the way people sort them: ignoring case, with numbers in order (`2` before `10`).
pub fn siblings(path: &Path) -> Vec<std::path::PathBuf> {
    let Some(folder) = path.parent().filter(|f| !f.as_os_str().is_empty()) else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut files: Vec<std::path::PathBuf> = entries
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file() || t.is_symlink()))
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .collect();
    files.sort_by_cached_key(|p| natural_key(&p.file_name().unwrap_or_default().to_string_lossy()));
    files
}

/// A key that sorts names ignoring case, with each run of digits compared as a number.
pub(crate) fn natural_key(name: &str) -> Vec<(u8, u128, String)> {
    let mut key = Vec::new();
    let mut chars = name.chars().peekable();
    while let Some(&c) = chars.peek() {
        let digits = c.is_ascii_digit();
        let mut run = String::new();
        while let Some(&c) = chars.peek().filter(|c| c.is_ascii_digit() == digits) {
            run.push(c);
            chars.next();
        }
        key.push(if digits {
            (0, run.parse().unwrap_or(u128::MAX), String::new())
        } else {
            (1, 0, run.to_lowercase())
        });
    }
    key
}

/// What highlights `path`'s text: its extension, else its name (`Makefile`, `Dockerfile`),
/// which the highlighter matches against the languages it knows.
pub fn syntax(path: &Path) -> String {
    path.extension()
        .or_else(|| path.file_name())
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// `len` bytes for people: `12 bytes`, `3.4 KB`, `1.2 MB`.
pub fn size(len: u64) -> String {
    const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
    if len < 1024 {
        return format!("{len} bytes");
    }
    let mut value = len as f64 / 1024.0;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

/// Where a link in a Markdown file at `file` goes, when it's another file: relative to the
/// file's folder (or absolute, or `file://`), without its `#section` or `?query`, with `%20`
/// and the like decoded. `None` for a web address, a `#section` of the same file, or any other
/// scheme.
pub fn linked_file(file: &Path, link: &str) -> Option<std::path::PathBuf> {
    let link = link.trim();
    let target = match link.strip_prefix("file://") {
        // `file:///C:/x` on Windows: drop the slash before the drive letter.
        Some(rest) if cfg!(windows) && rest.get(2..3) == Some(":") => &rest[1..],
        Some(rest) => rest,
        None if link.starts_with('#') => return None,
        // A scheme (`https:`, `mailto:`), but not a Windows drive letter (`C:`).
        None if link
            .split_once(':')
            .is_some_and(|(scheme, _)| scheme.len() > 1 && !scheme.contains(['/', '\\'])) =>
        {
            return None;
        }
        None => link,
    };
    let target = target.split(['#', '?']).next().unwrap_or_default();
    if target.is_empty() {
        return None;
    }
    let target = std::path::PathBuf::from(percent_decode(target));
    Some(if target.is_absolute() {
        target
    } else {
        file.parent().unwrap_or(Path::new("")).join(target)
    })
}

/// `text` with each `%xx` escape turned back into its byte (left as typed when it isn't one).
pub(crate) fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        if bytes[i] == b'%'
            && let (Some(high), Some(low)) = (
                bytes.get(i + 1).copied().and_then(hex),
                bytes.get(i + 2).copied().and_then(hex),
            )
        {
            out.push((high * 16 + low) as u8);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A folder in the temp directory for one test, emptied; the test deletes it when it
    /// passes.
    fn scratch(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("macaroni-file-view-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn kinds_come_from_the_extension() {
        assert_eq!(Kind::of(Path::new("README.md")), Kind::Markdown);
        assert_eq!(Kind::of(Path::new("a/b/Notes.MARKDOWN")), Kind::Markdown);
        assert_eq!(Kind::of(Path::new("logo.PNG")), Kind::Image);
        assert_eq!(Kind::of(Path::new("icon.svg")), Kind::Svg);
        assert_eq!(Kind::of(Path::new("main.rs")), Kind::Text);
        assert_eq!(Kind::of(Path::new("clip.MP4")), Kind::Media(Media::Video));
        assert_eq!(Kind::of(Path::new("song.flac")), Kind::Media(Media::Audio));
        assert_eq!(Kind::of(Path::new("spin.gif")), Kind::Media(Media::Picture));
        assert_eq!(
            Kind::of(Path::new("flow.mmd")),
            Kind::Diagram(crate::viewers::Extension::Mermaid)
        );
        assert_eq!(Kind::of(Path::new("Makefile")), Kind::Text);
        assert_eq!(syntax(Path::new("src/main.rs")), "rs");
        assert_eq!(syntax(Path::new("Dockerfile")), "Dockerfile");
    }

    #[test]
    fn text_is_decoded_and_binary_is_told_apart() {
        assert_eq!(decode(b"\xEF\xBB\xBFhi").as_deref(), Some("hi"));
        assert_eq!(decode(b"\xFF\xFEh\0i\0").as_deref(), Some("hi"));
        assert_eq!(decode(b"\xFE\xFF\0h\0i").as_deref(), Some("hi"));
        assert_eq!(decode(b"caf\xE9").as_deref(), Some("caf\u{FFFD}"));
        assert_eq!(decode(b"MZ\0\0\x03"), None);
    }

    #[test]
    fn files_are_read_cut_and_stamped() {
        let dir = scratch("read");
        let file = dir.join("notes.txt");
        std::fs::write(&file, "one\r\ntwo\n").unwrap();
        let (stamp, read) = read(&file, Kind::Text);
        assert!(stamp.is_some());
        assert!(matches!(read, Read::Text { ref text, cut: false } if text == "one\ntwo\n"));

        let big = dir.join("big.log");
        std::fs::write(&big, "x".repeat(MAX_TEXT + 10)).unwrap();
        let Read::Text { text, cut } = super::read(&big, Kind::Text).1 else {
            panic!("not text");
        };
        assert!(cut);
        assert_eq!(text.len(), MAX_TEXT);

        std::fs::write(&file, "one\ntwo\nthree\n").unwrap();
        assert_ne!(super::stamp(&file), stamp, "a longer file has a new stamp");

        assert!(matches!(
            super::read(&dir.join("gone.md"), Kind::Markdown).1,
            Read::Missing
        ));
        assert!(matches!(super::read(&dir, Kind::Text).1, Read::Folder));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn markdown_links_lead_to_files_beside_it() {
        let file = Path::new("/repo/docs/README.md");
        assert_eq!(
            linked_file(file, "guide.md#setup"),
            Some(Path::new("/repo/docs").join("guide.md"))
        );
        assert_eq!(
            linked_file(file, "../My%20Notes.md"),
            Some(Path::new("/repo/docs").join("../My Notes.md"))
        );
        assert_eq!(linked_file(file, "https://example.com/a.md"), None);
        assert_eq!(linked_file(file, "mailto:me@example.com"), None);
        assert_eq!(linked_file(file, "#usage"), None);
        if cfg!(windows) {
            assert_eq!(
                linked_file(file, "file:///C:/notes/a.md"),
                Some(std::path::PathBuf::from("C:/notes/a.md"))
            );
            assert_eq!(
                linked_file(file, "C:\\notes\\a.md"),
                Some(std::path::PathBuf::from("C:\\notes\\a.md"))
            );
        } else {
            assert_eq!(
                linked_file(file, "file:///notes/a.md"),
                Some(std::path::PathBuf::from("/notes/a.md"))
            );
        }
        assert_eq!(size(12), "12 bytes");
        assert_eq!(size(3 * 1024 + 512), "3.5 KB");
    }

    #[test]
    fn siblings_are_the_folders_files_in_natural_order() {
        let dir = scratch("siblings");
        for name in ["b.txt", "A.md", "photo10.png", "photo2.png"] {
            std::fs::write(dir.join(name), "x").unwrap();
        }
        std::fs::create_dir(dir.join("folder")).unwrap();
        let names: Vec<String> = siblings(&dir.join("b.txt"))
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["A.md", "b.txt", "photo2.png", "photo10.png"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
