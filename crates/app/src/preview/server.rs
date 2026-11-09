//! The HTTP server file tabs' pages load from: 127.0.0.1 on a free port, every path behind a
//! random token, started the first time a page is published ([`get`]).
//!
//! Routes under `/<token>`:
//!
//! - `page/<id>`: the page (the frame around the last published body), with its content
//!   security policy and a fresh nonce; `page/<id>/body` the body alone and
//!   `page/<id>/version` its version and frame fingerprint, which the page polls;
//!   `POST page/<id>/link` a clicked link, sent to the app as [`BrowserMessage::Link`].
//! - `app/preview.js`, `app/preview.css`: the page's own script and style.
//! - `ext/<viewer>/<path>`: a downloaded viewer's files.
//! - `fs/<absolute path>`: a file the document links to (the page's `<base>` is its folder
//!   here, so relative links and `../` resolve as on disk).
//!
//! The token keeps other local users' programs and web pages out (they can't guess it); the
//! policy keeps scripts in a document from running, so they can't use it either. One thread
//! per connection, each closed after one response. Files are streamed from disk and answer
//! `Range` requests, which video and audio players need to seek.

use std::collections::HashMap;
use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use iced::futures::channel::mpsc::UnboundedSender;
use macaroni_core::BrowserId;

use super::{Look, Page};
use crate::browser::BrowserMessage;

/// The largest request body read (a link).
const MAX_BODY: usize = 64 * 1024;

/// The server, once started (or why it couldn't start).
static SERVER: OnceLock<Result<Server, String>> = OnceLock::new();

/// Where links clicked in pages go.
type Links = UnboundedSender<(BrowserId, BrowserMessage)>;

/// The running server.
pub struct Server {
    /// The port on 127.0.0.1.
    port: u16,
    /// `/<token>`, which every path starts with.
    prefix: String,
    /// What the connection threads share with the app.
    shared: Arc<Shared>,
}

/// The server's state.
#[derive(Default)]
struct Shared {
    /// Each page's content and version, by its browser's id.
    pages: Mutex<HashMap<u64, Entry>>,
    /// Where links go, once the app's browser channel exists.
    links: Mutex<Option<Links>>,
}

/// One page.
struct Entry {
    /// What it shows.
    page: Page,
    /// Bumped whenever the body changes.
    version: u64,
}

/// The server, starting it the first time. Fails (every time after) if it can't listen.
pub fn get() -> Result<&'static Server, String> {
    SERVER
        .get_or_init(Server::start)
        .as_ref()
        .map_err(Clone::clone)
}

/// The server if it's running; `None` before the first page.
pub fn running() -> Option<&'static Server> {
    SERVER.get().and_then(|s| s.as_ref().ok())
}

/// `bytes` random bytes in hex.
fn random_hex(bytes: usize) -> String {
    let mut buffer = vec![0u8; bytes];
    // The OS's generator doesn't fail on the platforms macaroni runs on; if it did, the
    // clock still makes the token hard to guess from another user's session.
    if getrandom::fill(&mut buffer).is_err() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        for (i, byte) in buffer.iter_mut().enumerate() {
            *byte = (nanos >> ((i % 16) * 8)) as u8;
        }
    }
    buffer.iter().map(|b| format!("{b:02x}")).collect()
}

impl Server {
    /// Listens on a free port and starts the accepting thread.
    fn start() -> Result<Self, String> {
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .map_err(|e| format!("Couldn't start the page server: {e}"))?;
        let port = listener
            .local_addr()
            .map_err(|e| format!("Couldn't start the page server: {e}"))?
            .port();
        let prefix = format!("/{}", random_hex(16));
        let shared = Arc::new(Shared::default());
        let (thread_shared, thread_prefix) = (shared.clone(), prefix.clone());
        std::thread::Builder::new()
            .name("preview-server".into())
            .spawn(move || {
                for stream in listener.incoming().flatten() {
                    let (shared, prefix) = (thread_shared.clone(), thread_prefix.clone());
                    let _ = std::thread::Builder::new()
                        .name("preview-request".into())
                        .spawn(move || serve(stream, &shared, &prefix));
                }
            })
            .map_err(|e| format!("Couldn't start the page server: {e}"))?;
        Ok(Self {
            port,
            prefix,
            shared,
        })
    }

    /// The server's address and token, which every URL it serves starts with.
    fn origin(&self) -> String {
        format!("http://127.0.0.1:{}{}", self.port, self.prefix)
    }

    /// The address page `id` loads.
    pub fn url(&self, id: BrowserId) -> String {
        format!("{}/page/{}", self.origin(), id.0)
    }

    /// Whether `url` is page `id` itself (an in-page `#section` included).
    pub fn is_page(&self, id: BrowserId, url: &str) -> bool {
        url.strip_prefix(&self.url(id))
            .is_some_and(|rest| rest.is_empty() || rest.starts_with(['#', '?']))
    }

    /// The file a URL of this server's `fs/` route stands for, if it's one.
    pub fn file_of(&self, url: &str) -> Option<PathBuf> {
        let rest = url.strip_prefix(&format!("{}/fs/", self.origin()))?;
        file_path(rest.split(['#', '?']).next().unwrap_or_default())
    }

    /// Sends links clicked in pages to `links` from now on.
    pub fn connect(&self, links: Links) {
        if let Ok(mut slot) = self.shared.links.lock() {
            *slot = Some(links);
        }
    }

    /// Shows `page` as page `id`, bumping its version (which the page sees and swaps in)
    /// when the body changed.
    pub fn publish(&self, id: BrowserId, page: Page) {
        let Ok(mut pages) = self.shared.pages.lock() else {
            return;
        };
        match pages.get_mut(&id.0) {
            Some(entry) => {
                if entry.page.body != page.body {
                    entry.version += 1;
                }
                entry.page = page;
            }
            None => {
                pages.insert(id.0, Entry { page, version: 1 });
            }
        }
    }

    /// The look page `id` was last published with.
    pub fn look(&self, id: BrowserId) -> Option<Look> {
        let pages = self.shared.pages.lock().ok()?;
        pages.get(&id.0).map(|e| e.page.look.clone())
    }

    /// Changes page `id`'s look; the page notices its frame changed and reloads.
    pub fn restyle(&self, id: BrowserId, look: Look) {
        if let Ok(mut pages) = self.shared.pages.lock()
            && let Some(entry) = pages.get_mut(&id.0)
        {
            entry.page.look = look;
        }
    }

    /// Drops page `id` (its tab closed).
    pub fn forget(&self, id: BrowserId) {
        if let Ok(mut pages) = self.shared.pages.lock() {
            pages.remove(&id.0);
        }
    }
}

/// A request's method, path (without its query) and body.
struct Request {
    /// `GET`, `POST`...
    method: String,
    /// The path, percent-encoded as sent.
    path: String,
    /// The body (a link's text).
    body: Vec<u8>,
    /// The `Range` header's value, if any.
    range: Option<String>,
}

/// Reads one request.
fn read_request(stream: &TcpStream) -> Option<Request> {
    let mut reader = BufReader::new(stream.try_clone().ok()?);
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    let mut parts = line.split_whitespace();
    let method = parts.next()?.to_owned();
    let target = parts.next()?;
    let path = target
        .split(['?', '#'])
        .next()
        .unwrap_or_default()
        .to_owned();
    let mut length = 0;
    let mut range = None;
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header).ok()? == 0 || header.trim().is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':') {
            let name = name.trim();
            if name.eq_ignore_ascii_case("content-length") {
                length = value.trim().parse().ok()?;
            } else if name.eq_ignore_ascii_case("range") {
                range = Some(value.trim().to_owned());
            }
        }
    }
    let mut body = vec![0; length.min(MAX_BODY)];
    reader.read_exact(&mut body).ok()?;
    Some(Request {
        method,
        path,
        body,
        range,
    })
}

/// A response's status, type, extra headers and body.
struct Response {
    /// `200 OK` and so on.
    status: &'static str,
    /// The content type.
    kind: &'static str,
    /// Further headers, each `Name: value`.
    headers: Vec<String>,
    /// The body.
    body: Body,
}

/// What a response sends after its headers.
enum Body {
    /// These bytes.
    Bytes(Vec<u8>),
    /// `len` bytes of a file from `start`, streamed.
    File {
        /// The open file.
        file: std::fs::File,
        /// Where the bytes start.
        start: u64,
        /// How many.
        len: u64,
    },
}

impl Body {
    /// How many bytes it sends.
    fn len(&self) -> u64 {
        match self {
            Self::Bytes(bytes) => bytes.len() as u64,
            Self::File { len, .. } => *len,
        }
    }
}

impl Response {
    /// A 200 with `body` of type `kind`, cached for `cache` (`no-store` for live content).
    fn ok(kind: &'static str, body: impl Into<Vec<u8>>, cache: &str) -> Self {
        Self {
            status: "200 OK",
            kind,
            headers: vec![format!("Cache-Control: {cache}")],
            body: Body::Bytes(body.into()),
        }
    }

    /// An empty response with `status`.
    fn status(status: &'static str) -> Self {
        Self {
            status,
            kind: "text/plain",
            headers: Vec::new(),
            body: Body::Bytes(Vec::new()),
        }
    }
}

/// Answers one connection.
fn serve(stream: TcpStream, shared: &Shared, prefix: &str) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    let Some(request) = read_request(&stream) else {
        return;
    };
    let response = match request.path.strip_prefix(prefix) {
        Some(route) if route.starts_with('/') => answer(&request, &route[1..], shared, prefix),
        _ => Response::status("404 Not Found"),
    };
    let mut stream = stream;
    let mut head = format!(
        "HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nX-Content-Type-Options: \
         nosniff\r\nConnection: close\r\n",
        response.status,
        response.kind,
        response.body.len()
    );
    for header in &response.headers {
        head.push_str(header);
        head.push_str("\r\n");
    }
    head.push_str("\r\n");
    if stream.write_all(head.as_bytes()).is_err() {
        return;
    }
    // A player that has what it needs closes the connection mid-file; the error is expected.
    let _ = match response.body {
        Body::Bytes(bytes) => stream.write_all(&bytes),
        Body::File {
            mut file,
            start,
            len,
        } => {
            use std::io::Seek as _;
            file.seek(std::io::SeekFrom::Start(start))
                .and_then(|_| std::io::copy(&mut file.take(len), &mut stream).map(|_| ()))
        }
    };
}

/// The response for `route` (the path after the token).
fn answer(request: &Request, route: &str, shared: &Shared, prefix: &str) -> Response {
    let (head, rest) = route.split_once('/').unwrap_or((route, ""));
    match (request.method.as_str(), head) {
        (_, "page") => page_route(request, rest, shared, prefix),
        ("GET", "app") => match rest {
            "preview.js" => Response::ok("text/javascript", super::SCRIPT, "max-age=86400"),
            "preview.css" => Response::ok("text/css", super::STYLE, "max-age=86400"),
            _ => Response::status("404 Not Found"),
        },
        ("GET", "ext") => {
            let (viewer, path) = rest.split_once('/').unwrap_or((rest, ""));
            let viewer = crate::viewers::Extension::ALL
                .into_iter()
                .find(|e| e.id() == viewer);
            match (viewer, relative_path(path)) {
                (Some(viewer), Some(path)) => file(
                    &viewer.folder().join(path),
                    request.range.as_deref(),
                    "max-age=86400",
                ),
                _ => Response::status("404 Not Found"),
            }
        }
        ("GET", "fs") => match file_path(rest) {
            Some(path) => file(&path, request.range.as_deref(), "no-cache"),
            None => Response::status("404 Not Found"),
        },
        _ => Response::status("404 Not Found"),
    }
}

/// `page/<id>` and what's under it.
fn page_route(request: &Request, rest: &str, shared: &Shared, prefix: &str) -> Response {
    let (id, what) = rest.split_once('/').unwrap_or((rest, ""));
    let Ok(id) = id.parse::<u64>() else {
        return Response::status("404 Not Found");
    };
    if (request.method.as_str(), what) == ("POST", "link") {
        let link = String::from_utf8_lossy(&request.body).into_owned();
        if let Ok(links) = shared.links.lock()
            && let Some(links) = links.as_ref()
        {
            let _ = links.unbounded_send((BrowserId(id), BrowserMessage::Link(link)));
        }
        return Response::status("204 No Content");
    }
    let Ok(pages) = shared.pages.lock() else {
        return Response::status("500 Internal Server Error");
    };
    let Some(entry) = pages.get(&id) else {
        return Response::status("404 Not Found");
    };
    match what {
        "" => {
            let nonce = random_hex(16);
            let page_path = format!("{prefix}/page/{id}");
            let html = format!(
                "{}{}{}",
                super::head(&entry.page, prefix),
                entry.page.body,
                super::tail(&entry.page, prefix, &page_path, entry.version, &nonce)
            );
            let mut response = Response::ok("text/html; charset=utf-8", html, "no-store");
            response.headers.push(format!(
                "Content-Security-Policy: {}",
                super::policy(&nonce)
            ));
            response
        }
        "body" => Response::ok(
            "text/html; charset=utf-8",
            entry.page.body.clone(),
            "no-store",
        ),
        "version" => Response::ok(
            "text/plain",
            format!(
                "{} {} {}",
                entry.version,
                entry.page.frame_sig(),
                entry.page.document_sig()
            ),
            "no-store",
        ),
        _ => Response::status("404 Not Found"),
    }
}

/// A path inside a viewer's folder from its URL path: decoded, with no `..` or root.
fn relative_path(url_path: &str) -> Option<PathBuf> {
    let mut path = PathBuf::new();
    for part in url_path.split('/').filter(|p| !p.is_empty()) {
        let part = crate::file_view::percent_decode(part);
        if part == ".." || part == "." || part.contains(['/', '\\', ':']) {
            return None;
        }
        path.push(part);
    }
    (!path.as_os_str().is_empty()).then_some(path)
}

/// The absolute file a `fs/` URL path stands for: `C:/x/y` on Windows, `/x/y` elsewhere.
fn file_path(url_path: &str) -> Option<PathBuf> {
    let parts: Vec<String> = url_path
        .split('/')
        .filter(|p| !p.is_empty())
        .map(crate::file_view::percent_decode)
        .collect();
    if parts.iter().any(|p| p == ".." || p.contains(['/', '\\'])) {
        return None;
    }
    let joined = parts.join("/");
    let path = if cfg!(windows) {
        PathBuf::from(joined)
    } else {
        PathBuf::from(format!("/{joined}"))
    };
    path.is_absolute().then_some(path)
}

/// A file, streamed: the whole of it, or the part `range` (a `Range` header) asks for; 404
/// when it isn't a readable file, 416 when the range is outside it.
fn file(path: &std::path::Path, range: Option<&str>, cache: &str) -> Response {
    let Some((file, total)) = std::fs::File::open(path).ok().and_then(|f| {
        let meta = f.metadata().ok().filter(std::fs::Metadata::is_file)?;
        Some((f, meta.len()))
    }) else {
        return Response::status("404 Not Found");
    };
    let mut response = Response::ok(content_type(path), Vec::new(), cache);
    response.headers.push("Accept-Ranges: bytes".to_owned());
    let (start, len) = match range.map(|r| byte_range(r, total)) {
        None => (0, total),
        Some(Some((start, end))) => {
            response.status = "206 Partial Content";
            response
                .headers
                .push(format!("Content-Range: bytes {start}-{end}/{total}"));
            (start, end - start + 1)
        }
        Some(None) => {
            let mut response = Response::status("416 Range Not Satisfiable");
            response
                .headers
                .push(format!("Content-Range: bytes */{total}"));
            return response;
        }
    };
    response.body = Body::File { file, start, len };
    response
}

/// The first byte range in a `Range` header (`bytes=a-b`, `bytes=a-`, `bytes=-n`) as the first
/// and last byte, inclusive, clamped to a file of `total` bytes; `None` when it can't be met.
fn byte_range(header: &str, total: u64) -> Option<(u64, u64)> {
    let spec = header.strip_prefix("bytes=")?.split(',').next()?.trim();
    let (first, last) = spec.split_once('-')?;
    let (start, end) = match (first.trim(), last.trim()) {
        ("", suffix) => {
            let n: u64 = suffix.parse().ok()?;
            (total.checked_sub(n.min(total))?, total.checked_sub(1)?)
        }
        (first, "") => (first.parse().ok()?, total.checked_sub(1)?),
        (first, last) => (
            first.parse().ok()?,
            last.parse::<u64>().ok()?.min(total.checked_sub(1)?),
        ),
    };
    (start <= end && start < total).then_some((start, end))
}

/// The content type for `path`'s extension.
fn content_type(path: &std::path::Path) -> &'static str {
    let extension = path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    match extension.as_str() {
        "js" | "mjs" => "text/javascript",
        "css" => "text/css",
        "json" => "application/json",
        "wasm" => "application/wasm",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "bmp" => "image/bmp",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "heic" | "heif" => "image/heic",
        "jxl" => "image/jxl",
        "apng" => "image/apng",
        "mp4" | "m4v" => "video/mp4",
        "webm" => "video/webm",
        "mov" => "video/quicktime",
        "ogv" => "video/ogg",
        "mkv" => "video/x-matroska",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "ogg" | "oga" | "opus" => "audio/ogg",
        "flac" => "audio/flac",
        "m4a" => "audio/mp4",
        "aac" => "audio/aac",
        "weba" => "audio/webm",
        "aif" | "aiff" => "audio/aiff",
        "txt" | "md" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges_are_read_like_browsers_send_them() {
        assert_eq!(byte_range("bytes=0-", 100), Some((0, 99)));
        assert_eq!(byte_range("bytes=10-19", 100), Some((10, 19)));
        assert_eq!(byte_range("bytes=90-500", 100), Some((90, 99)));
        assert_eq!(byte_range("bytes=-10", 100), Some((90, 99)));
        assert_eq!(byte_range("bytes=100-", 100), None);
        assert_eq!(byte_range("bytes=0-", 0), None);
        assert_eq!(byte_range("lines=1-2", 100), None);
    }

    #[test]
    fn paths_stay_where_they_belong() {
        assert_eq!(
            relative_path("dist/katex.min.js"),
            Some(PathBuf::from("dist").join("katex.min.js"))
        );
        assert_eq!(relative_path("../secret"), None);
        assert_eq!(relative_path("%2E%2E/secret"), None);
        assert_eq!(relative_path("a/C:/x"), None);
        assert_eq!(file_path("x/../y"), None);
        if cfg!(windows) {
            assert_eq!(
                file_path("C:/Users/me/my%20docs/a.png"),
                Some(PathBuf::from("C:/Users/me/my docs/a.png"))
            );
            assert_eq!(file_path("Users/me"), None);
        } else {
            assert_eq!(
                file_path("Users/me/a%20b.png"),
                Some(PathBuf::from("/Users/me/a b.png"))
            );
        }
    }

    #[test]
    fn pages_are_served_behind_the_token() {
        let server = get().unwrap();
        let id = BrowserId(u64::MAX - 7);
        let page = Page {
            document: std::env::temp_dir().join("page.md"),
            body: "<p>hello</p>".to_owned(),
            look: Look {
                dark: false,
                text: iced::Color::BLACK,
                muted: iced::Color::BLACK,
                background: iced::Color::WHITE,
                sunken: iced::Color::WHITE,
                border: iced::Color::BLACK,
                link: iced::Color::BLACK,
                size: 14.0,
                code_size: 12.0,
                mono: "Mono".to_owned(),
            },
            installed: Vec::new(),
            whole: false,
        };
        server.publish(id, page.clone());
        let get = |path: &str| {
            let mut stream = TcpStream::connect(("127.0.0.1", server.port)).unwrap();
            write!(stream, "GET {path} HTTP/1.1\r\nHost: x\r\n\r\n").unwrap();
            let mut text = String::new();
            stream.read_to_string(&mut text).unwrap();
            text
        };
        let page_path = format!("{}/page/{}", server.prefix, id.0);
        let html = get(&page_path);
        assert!(html.starts_with("HTTP/1.1 200"));
        assert!(html.contains("Content-Security-Policy: default-src 'none'"));
        assert!(html.contains("<p>hello</p>"));
        assert!(get(&format!("{page_path}/version")).ends_with(&format!(
            "1 {} {}",
            page.frame_sig(),
            page.document_sig()
        )));
        let video = std::env::temp_dir().join(format!("macaroni-range-{}.mp4", std::process::id()));
        std::fs::write(&video, b"0123456789").unwrap();
        let mut stream = TcpStream::connect(("127.0.0.1", server.port)).unwrap();
        write!(
            stream,
            "GET {}/fs/{} HTTP/1.1\r\nRange: bytes=2-4\r\n\r\n",
            server.prefix,
            video
                .to_string_lossy()
                .replace('\\', "/")
                .trim_start_matches('/')
        )
        .unwrap();
        let mut text = String::new();
        stream.read_to_string(&mut text).unwrap();
        std::fs::remove_file(&video).unwrap();
        assert!(text.starts_with("HTTP/1.1 206"), "{text}");
        assert!(text.contains("Content-Range: bytes 2-4/10"));
        assert!(text.contains("Content-Type: video/mp4"));
        assert!(text.ends_with("\r\n\r\n234"));
        let mut changed = page.clone();
        changed.body = "<p>bye</p>".to_owned();
        server.publish(id, changed);
        assert!(get(&format!("{page_path}/version")).contains("\r\n\r\n2 "));
        assert!(get(&format!("/nottoken/page/{}", id.0)).starts_with("HTTP/1.1 404"));
        assert!(get(&format!("{}/app/preview.js", server.prefix)).contains("MACARONI"));
        assert!(server.is_page(id, &format!("{}#intro", server.url(id))));
        assert!(!server.is_page(id, &format!("{}/fs/x", server.origin())));
        server.forget(id);
        assert!(get(&page_path).starts_with("HTTP/1.1 404"));
    }
}
