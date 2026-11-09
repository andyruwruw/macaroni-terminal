//! macaroni's own SSH client: a [`crate::Source`] whose program is a login shell on another
//! machine.
//!
//! [`open`] returns at once with the session's ends ([`crate::source::Opened`]); a task on the
//! shared tokio runtime ([`RUNTIME`]) makes the connection and then moves bytes between the
//! channel and those ends. Everything the user would see from `ssh` (progress, host key
//! questions, passphrase and password prompts, errors) is printed into the tab and answered
//! there ([`prompt::Term`]).
//!
//! - **Where to go** is the target combined with `~/.ssh/config` ([`config::resolve`]).
//! - **Connections are shared** ([`POOL`]): tabs to the same user, host, port and jump hosts use
//!   one authenticated connection, each with its own channel, so a second tab opens without
//!   signing in again. The connection closes when its last channel does.
//! - **ProxyJump** opens the connection through a `direct-tcpip` channel of the jump host's
//!   (itself pooled), outermost first.
//! - **Host keys** are checked against `~/.ssh/known_hosts`: an unknown host asks, a changed key
//!   refuses (`Client`'s `check_server_key`).
//! - **Signing in** tries the agent (OpenSSH's on Windows, else Pageant; `SSH_AUTH_SOCK`
//!   elsewhere), then the identity files (asking for a passphrase when one is encrypted), then
//!   keyboard-interactive, then a password ([`authenticate`]).
//!
//! The exit code is the remote shell's; a connection that fails or drops ends with 255, as
//! `ssh` does, which the app shows as disconnected.

mod config;
mod prompt;
#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex, Weak};
use std::time::Duration;

use russh::keys::{HashAlg, PrivateKeyWithHashAlg, PublicKey, PublicKeyOrCertificate};
use russh::{ChannelMsg, client};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::sync::oneshot;

use crate::grid::GridSize;
use crate::pipe::{ChunkReader, Input, InputWriter};
use crate::source::{Control, Opened};
use config::Resolved;
use prompt::{Next, Term};

/// Where to connect, as a profile names it. Unset fields come from `~/.ssh/config`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SshTarget {
    /// A host name or address, `user@host`, or a `Host` from `~/.ssh/config`.
    pub host: String,
    /// The login name, when `host` doesn't include one.
    pub user: Option<String>,
    /// The port.
    pub port: Option<u16>,
    /// A private key file, tried before the configured ones.
    pub identity_file: Option<String>,
    /// Hosts to connect through, `[user@]host[:port]`, comma-separated.
    pub proxy_jump: Option<String>,
    /// A command to run instead of the login shell (`tmux -CC new -A -s main`, say), in the
    /// same terminal.
    pub command: Option<String>,
}

/// The exit code for a connection that failed or dropped, as `ssh` uses.
const FAILED: u32 = 255;

/// How long a TCP connection may take to open.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

/// How often an idle connection is checked, and how many unanswered checks drop it.
const KEEPALIVE: (Duration, usize) = (Duration::from_secs(30), 3);

/// How many times a passphrase or password may be typed wrong.
const TRIES: usize = 3;

/// The runtime every SSH connection runs on: two worker threads are plenty for terminal
/// traffic.
static RUNTIME: LazyLock<tokio::runtime::Runtime> = LazyLock::new(|| {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .thread_name("ssh")
        .enable_all()
        .build()
        .expect("the SSH runtime starts")
});

/// Identifies a connection that can be shared: the user, address, port and jump hosts.
type PoolKey = (String, String, u16, Vec<String>);

/// One destination's connection, if it's live, behind a lock held while it's being made.
type PoolSlot = Arc<tokio::sync::Mutex<Weak<Connection>>>;

/// Live connections by destination, so two tabs opening at once share one instead of signing
/// in twice.
static POOL: LazyLock<Mutex<HashMap<PoolKey, PoolSlot>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// An authenticated connection. Sessions hold it while their channel is open.
struct Connection {
    /// russh's handle, which opens channels.
    handle: client::Handle<Client>,
    /// The jump host's connection this one goes through, kept open while this one is.
    _via: Option<Arc<Connection>>,
}

/// Opens a shell on `target` in a terminal of `size`. Returns at once; the connection is made on
/// [`RUNTIME`], printing its progress and questions into the session.
pub(crate) fn open(target: SshTarget, size: GridSize, env: &[(String, String)]) -> Opened {
    let (out_tx, out_rx) = std::sync::mpsc::channel();
    let (in_tx, in_rx) = tokio::sync::mpsc::unbounded_channel();
    let (kill_tx, kill_rx) = oneshot::channel();
    let (exit_tx, exit_rx) = std::sync::mpsc::channel();
    let term = Term::new(out_tx, in_rx, kill_rx, (size.cols.max(1), size.rows.max(1)));
    // Only variables a server might accept; macaroni's own socket and tab id mean nothing there.
    let env: Vec<(String, String)> = env
        .iter()
        .filter(|(k, _)| k == "LANG" || k.starts_with("LC_"))
        .cloned()
        .collect();
    RUNTIME.spawn(async move {
        let code = run(target, term, env).await;
        let _ = exit_tx.send(code);
    });
    Opened {
        reader: Box::new(ChunkReader::new(out_rx)),
        writer: Box::new(InputWriter(in_tx.clone())),
        control: Box::new(SshControl {
            input: in_tx,
            kill: Mutex::new(Some(kill_tx)),
        }),
        waiter: Box::new(move || exit_rx.recv().ok().flatten()),
    }
}

/// What a session keeps of its SSH task.
struct SshControl {
    /// Where resizes go, with the input.
    input: UnboundedSender<Input>,
    /// Ends the task (and closes the channel) when dropped or sent.
    kill: Mutex<Option<oneshot::Sender<()>>>,
}

impl Control for SshControl {
    fn resize(&self, size: GridSize) {
        let _ = self
            .input
            .send(Input::Resize(size.cols.max(1), size.rows.max(1)));
    }
}

impl Drop for SshControl {
    fn drop(&mut self) {
        // The reader and sync threads hold the writer, so the input channel never closes on its
        // own: the task is told to end instead.
        if let Ok(mut kill) = self.kill.lock()
            && let Some(kill) = kill.take()
        {
            let _ = kill.send(());
        }
    }
}

/// Connects, opens a shell and pumps it until it ends; the exit code.
async fn run(target: SshTarget, mut term: Term, env: Vec<(String, String)>) -> Option<u32> {
    let resolved = config::resolve(&target);
    let connection = match connect(&resolved, &mut term).await {
        Ok(connection) => connection,
        Err(error) => {
            if !error.is_empty() {
                term.print(&format!("\x1b[31m{error}\x1b[0m\n"));
            }
            return Some(FAILED);
        }
    };
    match shell(&connection, &mut term, &env, target.command.as_deref()).await {
        Ok(code) => code,
        Err(error) => {
            term.print(&format!("\x1b[31m{error}\x1b[0m\n"));
            Some(FAILED)
        }
    }
}

/// A shared connection to `resolved`, made (through its jump hosts) if there's none.
/// `Err("")` means the user cancelled, so there's nothing more to say.
async fn connect(resolved: &Resolved, term: &mut Term) -> Result<Arc<Connection>, String> {
    let key = (
        resolved.user.clone(),
        resolved.hostname.clone(),
        resolved.port,
        resolved.proxy_jump.clone(),
    );
    let slot = POOL
        .lock()
        .map_err(|_| "SSH connections are unavailable".to_owned())?
        .entry(key)
        .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(Weak::new())))
        .clone();
    let mut slot = slot.lock().await;
    if let Some(live) = slot.upgrade()
        && !live.handle.is_closed()
    {
        return Ok(live);
    }
    let connection = Arc::new(Box::pin(new_connection(resolved, term)).await?);
    *slot = Arc::downgrade(&connection);
    Ok(connection)
}

/// Makes and authenticates a connection to `resolved`: straight there, or through the last of
/// its jump hosts (whose connection is made the same way, through the ones before it).
async fn new_connection(resolved: &Resolved, term: &mut Term) -> Result<Connection, String> {
    let config = Arc::new(client::Config {
        keepalive_interval: Some(KEEPALIVE.0),
        keepalive_max: KEEPALIVE.1,
        nodelay: true,
        ..client::Config::default()
    });
    let (notes_tx, notes) = tokio::sync::mpsc::unbounded_channel();
    let handler = Client {
        alias: resolved.alias.clone(),
        port: resolved.port,
        accepted: None,
        notes: notes_tx,
    };
    let (handle, via) = match resolved.proxy_jump.split_last() {
        None => {
            term.note(&format!(
                "Connecting to {}{}...",
                resolved.alias,
                port_suffix(resolved.port)
            ));
            let address = (resolved.hostname.as_str(), resolved.port);
            let stream =
                tokio::time::timeout(CONNECT_TIMEOUT, tokio::net::TcpStream::connect(address))
                    .await
                    .map_err(|_| format!("ssh: connect to {}: timed out", resolved.alias))?
                    .map_err(|e| format!("ssh: connect to {}: {e}", resolved.alias))?;
            let _ = stream.set_nodelay(true);
            (
                handshake(config, stream, handler, notes, resolved, term).await?,
                None,
            )
        }
        Some((last, before)) => {
            let mut jump = config::resolve(&config::jump_target(last));
            jump.proxy_jump = before.to_vec();
            let via = connect(&jump, term).await?;
            term.note(&format!(
                "Connecting to {}{} through {}...",
                resolved.alias,
                port_suffix(resolved.port),
                jump.alias
            ));
            let channel = via
                .handle
                .channel_open_direct_tcpip(
                    resolved.hostname.clone(),
                    u32::from(resolved.port),
                    "127.0.0.1",
                    0,
                )
                .await
                .map_err(|e| format!("ssh: {} can't reach {}: {e}", jump.alias, resolved.alias))?;
            (
                handshake(
                    config,
                    channel.into_stream(),
                    handler,
                    notes,
                    resolved,
                    term,
                )
                .await?,
                Some(via),
            )
        }
    };
    let mut handle = handle;
    authenticate(&mut handle, resolved, term).await?;
    Ok(Connection { handle, _via: via })
}

/// Runs the SSH handshake over `stream`, showing what the handler says about the host key in
/// `term` and answering its questions from there.
async fn handshake<S>(
    config: Arc<client::Config>,
    stream: S,
    handler: Client,
    mut notes: UnboundedReceiver<HostKeyNote>,
    resolved: &Resolved,
    term: &mut Term,
) -> Result<client::Handle<Client>, String>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let connecting = client::connect_stream(config, stream, handler);
    tokio::pin!(connecting);
    // The handler runs inside the handshake, but only this task can read the terminal.
    loop {
        tokio::select! {
            result = &mut connecting => {
                return result.map_err(|e| handshake_error(e, resolved));
            }
            Some(note) = notes.recv() => match note {
                HostKeyNote::Ask { text, answer } => {
                    term.print(&text);
                    let yes = term
                        .confirm("Are you sure you want to continue connecting (yes/no)? ")
                        .await;
                    let _ = answer.send(yes);
                }
                HostKeyNote::Warn(text) => term.print(&text),
            },
        }
    }
}

/// The message for a failed handshake.
fn handshake_error(error: russh::Error, resolved: &Resolved) -> String {
    match error {
        russh::Error::UnknownKey => format!(
            "ssh: {}: the host key wasn't accepted, so the connection was closed.",
            resolved.alias
        ),
        error => format!("ssh: {}: {error}", resolved.alias),
    }
}

/// What the handler has to say about a host key, shown by the connecting task.
enum HostKeyNote {
    /// An unknown key: show `text` and ask whether to trust it.
    Ask {
        /// What to print before asking.
        text: String,
        /// The answer.
        answer: oneshot::Sender<bool>,
    },
    /// A changed key: show the warning; the connection is refused.
    Warn(String),
}

/// russh's handler for one connection: checks the server's key.
struct Client {
    /// The name the host was asked for by, as known_hosts records it.
    alias: String,
    /// Its port.
    port: u16,
    /// The key accepted at the first exchange; later exchanges must show the same.
    accepted: Option<PublicKey>,
    /// Where to ask about an unknown key; the connecting task answers.
    notes: UnboundedSender<HostKeyNote>,
}

impl client::Handler for Client {
    type Error = russh::Error;

    /// Accepts a key `~/.ssh/known_hosts` has for the host; asks about one it doesn't (and
    /// records it on yes); refuses one that changed, as OpenSSH does.
    async fn check_server_key(
        &mut self,
        server: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        let key = match server {
            PublicKeyOrCertificate::PublicKey { key, .. } => key.clone(),
            PublicKeyOrCertificate::Certificate(cert) => {
                PublicKey::new(cert.public_key().clone(), "")
            }
        };
        if let Some(accepted) = &self.accepted {
            return Ok(accepted.key_data() == key.key_data());
        }
        let Some(known_hosts) = config::home().map(|h| h.join(".ssh").join("known_hosts")) else {
            return Ok(false);
        };
        let host = self.alias.clone();
        let fingerprint = key.fingerprint(HashAlg::Sha256);
        let algorithm = key.algorithm().as_str().to_owned();
        match russh::keys::check_known_hosts_path(&host, self.port, &key, &known_hosts) {
            Ok(true) => {}
            Ok(false) => {
                let (tx, rx) = oneshot::channel();
                let text = format!(
                    "The authenticity of host '{host}{}' can't be established.\n{algorithm} key fingerprint is {fingerprint}.\nThis key is not known by any other names.\n",
                    port_suffix(self.port)
                );
                let sent = self
                    .notes
                    .send(HostKeyNote::Ask { text, answer: tx })
                    .is_ok();
                if !sent || !rx.await.unwrap_or(false) {
                    return Ok(false);
                }
                if let Some(dir) = known_hosts.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                let _ = russh::keys::known_hosts::learn_known_hosts_path(
                    &host,
                    self.port,
                    &key,
                    &known_hosts,
                );
            }
            Err(russh::keys::Error::KeyChanged { line }) => {
                let text = format!(
                    "\x1b[31mWARNING: THE HOST KEY FOR '{host}' HAS CHANGED!\nSomeone could be eavesdropping on you right now, or the host key has just been changed.\nThe {algorithm} key sent is {fingerprint}.\nIt doesn't match line {line} of {}. Remove that line if the change is expected.\x1b[0m\n",
                    known_hosts.display()
                );
                let _ = self.notes.send(HostKeyNote::Warn(text));
                return Ok(false);
            }
            Err(_) => return Ok(false),
        }
        self.accepted = Some(key);
        Ok(true)
    }
}

/// Signs in as `resolved.user`: the agent's keys, the identity files, keyboard-interactive,
/// then a password, as far as the server allows each.
async fn authenticate(
    handle: &mut client::Handle<Client>,
    resolved: &Resolved,
    term: &mut Term,
) -> Result<(), String> {
    let user = resolved.user.clone();
    let failed = |e: russh::Error| format!("ssh: {}: {e}", resolved.alias);
    let rsa_hash = handle
        .best_supported_rsa_hash()
        .await
        .map_err(failed)?
        .flatten();

    if agent_auth(handle, &user, rsa_hash).await {
        return Ok(());
    }

    for path in resolved.identity_files.iter().filter(|p| p.is_file()) {
        let key = match russh::keys::load_secret_key(path, None) {
            Ok(key) => Some(key),
            Err(russh::keys::Error::KeyIsEncrypted) => {
                let mut loaded = None;
                for _ in 0..TRIES {
                    let Some(passphrase) = term
                        .ask(
                            &format!("Enter passphrase for key '{}': ", path.display()),
                            false,
                        )
                        .await
                    else {
                        return Err(String::new());
                    };
                    if let Ok(key) = russh::keys::load_secret_key(path, Some(&passphrase)) {
                        loaded = Some(key);
                        break;
                    }
                }
                loaded
            }
            Err(_) => None,
        };
        let Some(key) = key else { continue };
        let key = PrivateKeyWithHashAlg::new(Arc::new(key), rsa_hash);
        if handle
            .authenticate_publickey(user.clone(), key)
            .await
            .map_err(failed)?
            .success()
        {
            return Ok(());
        }
    }

    let mut answer = handle
        .authenticate_keyboard_interactive_start(user.clone(), None::<String>)
        .await
        .map_err(failed)?;
    loop {
        match answer {
            client::KeyboardInteractiveAuthResponse::Success => return Ok(()),
            client::KeyboardInteractiveAuthResponse::Failure { .. } => break,
            client::KeyboardInteractiveAuthResponse::InfoRequest {
                name,
                instructions,
                prompts,
            } => {
                for line in [name, instructions] {
                    if !line.trim().is_empty() {
                        term.print(&format!("{}\n", line.trim_end()));
                    }
                }
                let mut responses = Vec::new();
                for prompt in prompts {
                    let Some(reply) = term.ask(&prompt.prompt, prompt.echo).await else {
                        return Err(String::new());
                    };
                    responses.push(reply);
                }
                answer = handle
                    .authenticate_keyboard_interactive_respond(responses)
                    .await
                    .map_err(failed)?;
            }
        }
    }

    for _ in 0..TRIES {
        let Some(password) = term
            .ask(&format!("{user}@{}'s password: ", resolved.alias), false)
            .await
        else {
            return Err(String::new());
        };
        if handle
            .authenticate_password(user.clone(), password)
            .await
            .map_err(failed)?
            .success()
        {
            return Ok(());
        }
        term.print("Permission denied, please try again.\n");
    }
    Err(format!(
        "ssh: {user}@{}: permission denied.",
        resolved.alias
    ))
}

/// Tries each of the agent's keys; `true` once one is accepted. No agent, or none of its keys,
/// is `false`.
async fn agent_auth(
    handle: &mut client::Handle<Client>,
    user: &str,
    rsa_hash: Option<HashAlg>,
) -> bool {
    #[cfg(windows)]
    {
        if let Ok(mut agent) = russh::keys::agent::client::AgentClient::connect_named_pipe(
            r"\\.\pipe\openssh-ssh-agent",
        )
        .await
            && try_agent_keys(handle, user, rsa_hash, &mut agent).await
        {
            return true;
        }
        if let Ok(mut agent) = russh::keys::agent::client::AgentClient::connect_pageant().await
            && try_agent_keys(handle, user, rsa_hash, &mut agent).await
        {
            return true;
        }
        false
    }
    #[cfg(not(windows))]
    {
        match russh::keys::agent::client::AgentClient::connect_env().await {
            Ok(mut agent) => try_agent_keys(handle, user, rsa_hash, &mut agent).await,
            Err(_) => false,
        }
    }
}

/// Offers each key `agent` holds until one is accepted.
async fn try_agent_keys<S>(
    handle: &mut client::Handle<Client>,
    user: &str,
    rsa_hash: Option<HashAlg>,
    agent: &mut russh::keys::agent::client::AgentClient<S>,
) -> bool
where
    S: russh::keys::agent::client::AgentStream + Send + Unpin + 'static,
{
    let Ok(identities) = agent.request_identities().await else {
        return false;
    };
    for identity in identities {
        let key = identity.public_key().into_owned();
        if let Ok(result) = handle
            .authenticate_publickey_with(user, key, rsa_hash, agent)
            .await
            && result.success()
        {
            return true;
        }
    }
    false
}

/// Opens a shell channel (or runs `command` instead of the shell) with a terminal of the
/// session's size and pumps it until it ends; the remote exit code (`None` when it didn't send
/// one but closed cleanly).
async fn shell(
    connection: &Connection,
    term: &mut Term,
    env: &[(String, String)],
    command: Option<&str>,
) -> Result<Option<u32>, String> {
    let failed = |e: russh::Error| format!("ssh: {e}");
    let mut channel = connection
        .handle
        .channel_open_session()
        .await
        .map_err(failed)?;
    let (cols, rows) = term.size;
    channel
        .request_pty(
            true,
            "xterm-256color",
            u32::from(cols),
            u32::from(rows),
            0,
            0,
            &[],
        )
        .await
        .map_err(failed)?;
    // Servers only take what their `AcceptEnv` lists; refusals are fine.
    for (key, value) in env.iter().cloned().chain([
        ("COLORTERM".to_owned(), "truecolor".to_owned()),
        ("TERM_PROGRAM".to_owned(), "macaroni".to_owned()),
    ]) {
        let _ = channel.set_env(false, key, value).await;
    }
    match command {
        Some(command) => channel.exec(true, command).await.map_err(failed)?,
        None => channel.request_shell(true).await.map_err(failed)?,
    }
    let typeahead = std::mem::take(&mut term.typeahead);
    if !typeahead.is_empty() {
        let _ = channel.data_bytes(typeahead).await;
    }
    let mut code = None;
    loop {
        tokio::select! {
            message = channel.wait() => match message {
                Some(ChannelMsg::Data { data } | ChannelMsg::ExtendedData { data, .. }) => {
                    term.output(&data);
                }
                Some(ChannelMsg::ExitStatus { exit_status }) => code = Some(exit_status),
                Some(ChannelMsg::Close) | None => {
                    // Closed without a status: the connection dropped.
                    return Ok(code.or(Some(FAILED)));
                }
                _ => {}
            },
            next = term.next() => match next {
                Next::Data(bytes) => {
                    if channel.data_bytes(bytes).await.is_err() {
                        return Ok(code.or(Some(FAILED)));
                    }
                }
                Next::Resized => {
                    let (cols, rows) = term.size;
                    let _ = channel.window_change(u32::from(cols), u32::from(rows), 0, 0).await;
                }
                Next::Closed => {
                    let _ = channel.close().await;
                    return Ok(code);
                }
            },
        }
    }
}

/// `:port` when it isn't 22, for messages.
fn port_suffix(port: u16) -> String {
    if port == 22 {
        String::new()
    } else {
        format!(":{port}")
    }
}
