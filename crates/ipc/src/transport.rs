//! The socket itself: where it lives ([`endpoint`]), opening it for the current user only
//! ([`listen`]), connecting ([`connect`]), and moving one JSON value per line ([`write_line`],
//! [`read_line`]).
//!
//! Built on `interprocess` local sockets: a named pipe in the Windows namespace, a socket file
//! elsewhere. The app's server (`crates/app/src/ipc.rs`) listens and runs a reader and a writer
//! thread per connection; `macaroni-cli` connects, writes a [`crate::Request`] and reads the
//! [`crate::Response`].

use std::io::{self, BufRead, Write};

use interprocess::local_socket::prelude::*;
use interprocess::local_socket::{GenericFilePath, GenericNamespaced, ListenerOptions, Name};
use serde::Serialize;
use serde::de::DeserializeOwned;

/// `&Stream` implements `Read` and `Write`, so a stream shared through an `Arc` can be read on
/// one thread and written on another.
pub use interprocess::local_socket::{Listener, Stream};

/// Environment variable the app sets in every terminal, and the CLI reads.
pub const SOCKET_ENV: &str = "MACARONI_SOCKET";
/// The tab a terminal belongs to, so CLI calls from inside it default to that tab.
pub const TAB_ENV: &str = "MACARONI_TAB_ID";

/// The socket to use: `MACARONI_SOCKET` if set, else a per-user default. On Windows a named
/// pipe name (`\\.\pipe\macaroni-<user>`), elsewhere a socket file in the runtime directory.
///
/// The user name comes from `USERNAME` or `USER` (else `user`), with anything but ASCII letters
/// and digits replaced by `_` so it's safe in a pipe name or path. The runtime directory is
/// `$XDG_RUNTIME_DIR`, else `$TMPDIR`, else `/tmp`.
pub fn endpoint() -> String {
    if let Some(endpoint) = std::env::var_os(SOCKET_ENV).filter(|e| !e.is_empty()) {
        return endpoint.to_string_lossy().into_owned();
    }
    let user = std::env::var("USERNAME")
        .or_else(|_| std::env::var("USER"))
        .unwrap_or_else(|_| "user".into());
    let user: String = user
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    if cfg!(windows) {
        format!("macaroni-{user}")
    } else {
        let dir = std::env::var("XDG_RUNTIME_DIR")
            .or_else(|_| std::env::var("TMPDIR"))
            .unwrap_or_else(|_| "/tmp".into());
        format!("{}/macaroni-{user}.sock", dir.trim_end_matches('/'))
    }
}

/// The `interprocess` name for an endpoint string: a namespaced pipe name on Windows (the
/// `\\.\pipe\` prefix is added for us), a file path elsewhere. Fails if the string isn't valid
/// for that kind of name.
fn name(endpoint: &str) -> io::Result<Name<'_>> {
    if cfg!(windows) {
        endpoint.to_ns_name::<GenericNamespaced>()
    } else {
        endpoint.to_fs_name::<GenericFilePath>()
    }
}

/// Listens on `endpoint`, accessible to the current user only: on Windows the pipe's ACL grants
/// only its owner, elsewhere the socket file is mode 0600. Fails if another macaroni already
/// answers there; a stale socket file left by a crashed instance is replaced.
///
/// The check is a trial [`connect`], which a live server accepts and then sees close at once.
pub fn listen(endpoint: &str) -> io::Result<Listener> {
    if connect(endpoint).is_ok() {
        return Err(io::Error::new(
            io::ErrorKind::AddrInUse,
            format!("another macaroni is already listening on {endpoint}"),
        ));
    }
    let options = ListenerOptions::new().name(name(endpoint)?);
    #[cfg(windows)]
    let options = {
        use interprocess::os::windows::local_socket::ListenerOptionsExt;
        use interprocess::os::windows::security_descriptor::SecurityDescriptor;
        // Protected DACL with a single entry: generic all for the owner (the user running us).
        // "Protected" stops inherited entries from widening it; without a descriptor, named pipes
        // get a default DACL that also lets other local accounts read.
        let sddl = widestring::U16CString::from_str("D:P(A;;GA;;;OW)")
            .map_err(|e| io::Error::other(e.to_string()))?;
        options.security_descriptor(SecurityDescriptor::deserialize(&sddl)?)
    };
    #[cfg(unix)]
    let options = {
        use interprocess::os::unix::local_socket::ListenerOptionsExt;
        // Overwriting is safe here: the trial connect above already showed nobody answers.
        options.mode(0o600).try_overwrite(true)
    };
    options.create_sync()
}

/// Waits for the next client on `listener` (blocking).
pub fn accept(listener: &Listener) -> io::Result<Stream> {
    listener.accept()
}

/// Connects to the app listening on `endpoint`. Fails (typically `NotFound`) when no macaroni
/// is running there.
pub fn connect(endpoint: &str) -> io::Result<Stream> {
    Stream::connect(name(endpoint)?)
}

/// Writes `value` as one line of JSON, then flushes. Compact JSON never contains a raw
/// newline, so the line break is an unambiguous message boundary.
pub fn write_line<W: Write, T: Serialize>(writer: &mut W, value: &T) -> io::Result<()> {
    let mut line = serde_json::to_vec(value).map_err(io::Error::other)?;
    line.push(b'\n');
    writer.write_all(&line)?;
    writer.flush()
}

/// Reads one line of JSON; `Ok(None)` at end of stream. A line that isn't valid JSON for `T` is
/// an `InvalidData` error.
pub fn read_line<R: BufRead, T: DeserializeOwned>(reader: &mut R) -> io::Result<Option<T>> {
    let mut line = String::new();
    if reader.read_line(&mut line)? == 0 {
        return Ok(None);
    }
    serde_json::from_str(line.trim_end())
        .map(Some)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

#[cfg(test)]
mod tests {
    use std::io::BufReader;

    use super::*;
    use crate::protocol::{Call, Request, Response};

    #[test]
    fn request_and_response_over_a_real_socket() {
        let endpoint = if cfg!(windows) {
            format!("macaroni-test-{}", std::process::id())
        } else {
            format!("/tmp/macaroni-test-{}.sock", std::process::id())
        };
        let listener = listen(&endpoint).expect("listen");
        assert!(
            listen(&endpoint).is_err(),
            "a second listener must not displace the first"
        );

        let server = std::thread::spawn(move || {
            let stream = listener.accept().expect("accept");
            let request: Request = read_line(&mut BufReader::new(&stream)).unwrap().unwrap();
            assert_eq!(request.call, Call::List);
            write_line(
                &mut &stream,
                &Response {
                    id: request.id,
                    result: Some(serde_json::json!({"ok": true})),
                    error: None,
                },
            )
            .unwrap();
        });

        let stream = connect(&endpoint).expect("connect");
        write_line(
            &mut &stream,
            &Request {
                id: 9,
                call: Call::List,
            },
        )
        .unwrap();
        let response: Response = read_line(&mut BufReader::new(&stream)).unwrap().unwrap();
        assert_eq!(response.id, 9);
        assert_eq!(response.result, Some(serde_json::json!({"ok": true})));
        server.join().unwrap();
    }
}
