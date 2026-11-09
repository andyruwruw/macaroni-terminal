//! The client against a small russh server on localhost: the host key question, a password,
//! a shell that echoes, its exit code, and a second session sharing the connection. The home
//! folder is a scratch one ([`super::config::TEST_HOME`]), so `~/.ssh` isn't touched.

use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use russh::server::{self, Auth, Msg, Session as ServerSession};
use russh::{Channel, ChannelId};

use super::SshTarget;
use crate::grid::{CursorShape, GridSize};
use crate::session::{Session, SessionConfig, SessionEvent};
use crate::source::Source;

/// A throwaway ed25519 host key, made for this test.
const HOST_KEY: &str = "-----BEGIN OPENSSH PRIVATE KEY-----
b3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAAAMwAAAAtzc2gtZW
QyNTUxOQAAACBOg10P8vlvFr0sCab6VFP2QHgI2QYU60Z6Ox3ofPRbDAAAAJDXM+ze1zPs
3gAAAAtzc2gtZWQyNTUxOQAAACBOg10P8vlvFr0sCab6VFP2QHgI2QYU60Z6Ox3ofPRbDA
AAAECpjoo6JRn1pF9HvObwmwLl90s0M7owUtILTboNheBhvk6DXQ/y+W8WvSwJpvpUU/ZA
eAjZBhTrRno7Heh89FsMAAAADW1hY2Fyb25pLXRlc3Q=
-----END OPENSSH PRIVATE KEY-----
";

/// The server's side of one connection: takes one password, greets each shell, echoes what's
/// typed, and ends the shell with code 3 on `exit`.
#[derive(Clone)]
struct Server {
    /// How many times a password was asked for, across connections.
    logins: Arc<Mutex<usize>>,
}

impl server::Server for Server {
    type Handler = Self;
    fn new_client(&mut self, _: Option<std::net::SocketAddr>) -> Self {
        self.clone()
    }
}

impl server::Handler for Server {
    type Error = russh::Error;

    async fn auth_password(&mut self, user: &str, password: &str) -> Result<Auth, Self::Error> {
        *self.logins.lock().unwrap() += 1;
        Ok(if user == "tester" && password == "secret" {
            Auth::Accept
        } else {
            Auth::reject()
        })
    }

    async fn channel_open_session(
        &mut self,
        _: Channel<Msg>,
        reply: server::ChannelOpenHandle,
        _: &mut ServerSession,
    ) -> Result<(), Self::Error> {
        reply.accept().await;
        Ok(())
    }

    async fn shell_request(
        &mut self,
        channel: ChannelId,
        session: &mut ServerSession,
    ) -> Result<(), Self::Error> {
        session.channel_success(channel)?;
        session.data(channel, b"welcome\r\n".to_vec())?;
        Ok(())
    }

    async fn data(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut ServerSession,
    ) -> Result<(), Self::Error> {
        if data.windows(4).any(|w| w == b"exit") {
            session.exit_status_request(channel, 3)?;
            session.eof(channel)?;
            session.close(channel)?;
        } else {
            session.data(channel, data.to_vec())?;
        }
        Ok(())
    }
}

/// Starts the server on a free localhost port; the port.
fn serve(logins: Arc<Mutex<usize>>) -> u16 {
    let key = russh::keys::decode_secret_key(HOST_KEY, None).unwrap();
    let config = Arc::new(server::Config {
        keys: vec![key],
        auth_rejection_time: Duration::from_millis(10),
        auth_rejection_time_initial: Some(Duration::ZERO),
        ..server::Config::default()
    });
    let listener = super::RUNTIME
        .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
        .unwrap();
    let port = listener.local_addr().unwrap().port();
    super::RUNTIME.spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let handler = Server {
                logins: logins.clone(),
            };
            let config = config.clone();
            tokio::spawn(async move {
                if let Ok(session) = server::run_stream(config, stream, handler).await {
                    let _ = session.await;
                }
            });
        }
    });
    port
}

/// A session to the test server, and its events.
fn connect(port: u16) -> (Session, mpsc::Receiver<SessionEvent>) {
    let (tx, rx) = mpsc::channel();
    let session = Session::spawn(
        SessionConfig {
            source: Source::Ssh(SshTarget {
                host: "127.0.0.1".into(),
                user: Some("tester".into()),
                port: Some(port),
                ..SshTarget::default()
            }),
            program: None,
            args: Vec::new(),
            cwd: None,
            size: GridSize {
                cols: 100,
                rows: 30,
                cell_width: 8,
                cell_height: 16,
            },
            scrollback: 100,
            env: Vec::new(),
            colors: None,
            cursor: CursorShape::Block,
            triggers: None,
        },
        move |event| {
            let _ = tx.send(event);
        },
    )
    .unwrap();
    (session, rx)
}

/// Waits until the session's text contains `text`; panics with what it has after a while.
fn wait_for(session: &Session, text: &str) {
    let start = Instant::now();
    while !session.all_text().contains(text) {
        assert!(
            start.elapsed() < Duration::from_secs(20),
            "never saw {text:?}; the terminal has:\n{}",
            session.all_text()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Waits for the session's exit code.
fn exit_code(events: &mpsc::Receiver<SessionEvent>) -> Option<u32> {
    loop {
        match events.recv_timeout(Duration::from_secs(20)) {
            Ok(SessionEvent::Exited(code)) => return code,
            Ok(_) => {}
            Err(e) => panic!("no exit: {e}"),
        }
    }
}

#[test]
fn connects_signs_in_and_shares_the_connection() {
    let home = std::env::temp_dir().join("macaroni-ssh-test-home");
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();
    let _ = super::config::TEST_HOME.set(home.clone());
    let logins = Arc::new(Mutex::new(0));
    let port = serve(logins.clone());

    let (first, first_events) = connect(port);
    wait_for(&first, "Are you sure you want to continue connecting");
    first.write(b"yes\r");
    wait_for(&first, "tester@127.0.0.1's password:");
    first.write(b"wrong\r");
    wait_for(&first, "Permission denied, please try again.");
    first.write(b"secret\r");
    wait_for(&first, "welcome");
    first.write(b"hello there");
    wait_for(&first, "hello there");
    let known = std::fs::read_to_string(home.join(".ssh").join("known_hosts")).unwrap();
    assert!(known.contains("ssh-ed25519"), "{known}");

    // A second tab: no question, no password, the same connection.
    let (second, second_events) = connect(port);
    wait_for(&second, "welcome");
    assert!(!second.all_text().contains("password"));
    assert_eq!(*logins.lock().unwrap(), 2);

    second.write(b"exit\r");
    assert_eq!(exit_code(&second_events), Some(3));
    first.write(b"exit\r");
    assert_eq!(exit_code(&first_events), Some(3));
}

#[test]
fn a_refused_connection_ends_with_255() {
    let home = std::env::temp_dir().join("macaroni-ssh-test-home");
    let _ = super::config::TEST_HOME.set(home);
    // Bound and dropped: nothing listens there now.
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let (session, events) = connect(port);
    assert_eq!(exit_code(&events), Some(super::FAILED));
    // Output can still arrive after the exit.
    wait_for(&session, "ssh: connect to 127.0.0.1");
}
