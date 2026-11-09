//! Reading `~/.ssh/config` for a destination, as OpenSSH would: `Host` blocks whose patterns
//! match (`*`, `?`, `!negated`), each keyword's first value winning, `Include` followed, `Match`
//! blocks skipped. Only what macaroni's client uses is read: `HostName`, `User`, `Port`,
//! `IdentityFile` (all of them, in order) and `ProxyJump`.
//!
//! What a profile sets itself wins over the file, as command-line options do for `ssh`.

use std::path::{Path, PathBuf};

use super::SshTarget;

/// Where and how to connect, after the profile and `~/.ssh/config` are combined.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Resolved {
    /// The name the profile or the command used: known_hosts and prompts show it.
    pub alias: String,
    /// The address to connect to.
    pub hostname: String,
    /// The login name.
    pub user: String,
    /// The port.
    pub port: u16,
    /// Private keys to offer, in order; the default ones when none are configured.
    pub identity_files: Vec<PathBuf>,
    /// Hosts to go through first, each `[user@]host[:port]`, outermost first.
    pub proxy_jump: Vec<String>,
}

/// What the config file said for one host.
#[derive(Debug, Default, PartialEq, Eq)]
struct Found {
    /// `HostName`.
    hostname: Option<String>,
    /// `User`.
    user: Option<String>,
    /// `Port`.
    port: Option<u16>,
    /// Every `IdentityFile`, in order.
    identity_files: Vec<String>,
    /// `ProxyJump`.
    proxy_jump: Option<String>,
}

/// Combines `target` with the user's `~/.ssh/config`.
pub(crate) fn resolve(target: &SshTarget) -> Resolved {
    let home = home();
    let text = home
        .as_ref()
        .map(|h| read_with_includes(&h.join(".ssh").join("config"), &h.join(".ssh"), 0))
        .unwrap_or_default();
    resolve_with(target, &text, home.as_deref())
}

/// [`resolve`] against config text already read, for tests.
fn resolve_with(target: &SshTarget, config: &str, home: Option<&Path>) -> Resolved {
    let (user_in_host, rest) = match target.host.split_once('@') {
        Some((user, host)) => (Some(user.to_owned()), host),
        None => (None, target.host.as_str()),
    };
    let alias = rest.trim().to_owned();
    let found = lookup(config, &alias);
    let user = target
        .user
        .clone()
        .or(user_in_host)
        .or(found.user)
        .or_else(local_user)
        .unwrap_or_else(|| "root".to_owned());
    let expand = |path: &str| expand_path(path, home, &alias, &user);
    let mut identity_files: Vec<PathBuf> = target
        .identity_file
        .iter()
        .map(|p| expand(p))
        .chain(found.identity_files.iter().map(|p| expand(p)))
        .collect();
    if identity_files.is_empty()
        && let Some(home) = home
    {
        identity_files = ["id_ed25519", "id_ecdsa", "id_rsa"]
            .iter()
            .map(|name| home.join(".ssh").join(name))
            .collect();
    }
    let proxy_jump = target
        .proxy_jump
        .clone()
        .or(found.proxy_jump)
        .filter(|j| !j.eq_ignore_ascii_case("none"))
        .map(|j| {
            j.split(',')
                .map(str::trim)
                .filter(|h| !h.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    Resolved {
        hostname: found.hostname.unwrap_or_else(|| alias.clone()),
        alias,
        user,
        port: target.port.or(found.port).unwrap_or(22),
        identity_files,
        proxy_jump,
    }
}

/// Splits a jump host, `[user@]host[:port]`, into a target.
pub(crate) fn jump_target(spec: &str) -> SshTarget {
    let (user, rest) = match spec.split_once('@') {
        Some((user, rest)) => (Some(user.to_owned()), rest),
        None => (None, spec),
    };
    let (host, port) = match rest.rsplit_once(':') {
        Some((host, port)) if !host.contains(':') => match port.parse() {
            Ok(port) => (host, Some(port)),
            Err(_) => (rest, None),
        },
        _ => (rest, None),
    };
    SshTarget {
        host: host.to_owned(),
        user,
        port,
        ..SshTarget::default()
    }
}

/// The settings for `alias` in `config`: the first value of each keyword among the `Host`
/// blocks that match it.
fn lookup(config: &str, alias: &str) -> Found {
    let mut found = Found::default();
    // Lines before the first `Host` apply to every host.
    let mut active = true;
    for line in config.lines() {
        let Some((keyword, value)) = split_line(line) else {
            continue;
        };
        match keyword.as_str() {
            "host" => active = host_matches(&value, alias),
            // Conditions macaroni doesn't evaluate: what follows is left out.
            "match" => active = false,
            _ if !active => {}
            "hostname" => {
                found.hostname.get_or_insert(value);
            }
            "user" => {
                found.user.get_or_insert(value);
            }
            "port" => {
                if found.port.is_none() {
                    found.port = value.parse().ok();
                }
            }
            "identityfile" => found.identity_files.push(value),
            "proxyjump" => {
                found.proxy_jump.get_or_insert(value);
            }
            _ => {}
        }
    }
    found
}

/// A config line's keyword (lowercase) and value, unquoted; `None` for blanks and comments.
fn split_line(line: &str) -> Option<(String, String)> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let at = line.find(|c: char| c.is_whitespace() || c == '=')?;
    let keyword = line[..at].to_ascii_lowercase();
    let value = line[at..]
        .trim_start_matches(|c: char| c.is_whitespace() || c == '=')
        .trim();
    let value = value
        .strip_prefix('"')
        .and_then(|v| v.strip_suffix('"'))
        .unwrap_or(value);
    Some((keyword, value.to_owned()))
}

/// Whether a `Host` line's patterns match `alias`: any positive pattern matches and no negated
/// one does.
fn host_matches(patterns: &str, alias: &str) -> bool {
    let mut matched = false;
    for pattern in patterns.split_whitespace() {
        match pattern.strip_prefix('!') {
            Some(negated) if glob(negated, alias) => return false,
            Some(_) => {}
            None => matched |= glob(pattern, alias),
        }
    }
    matched
}

/// `*` and `?` matching, case-insensitive, as host names are.
fn glob(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.to_lowercase().chars().collect();
    let t: Vec<char> = text.to_lowercase().chars().collect();
    let (mut pi, mut ti) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some((pi, ti));
            pi += 1;
        } else if let Some((sp, st)) = star {
            pi = sp + 1;
            ti = st + 1;
            star = Some((sp, st + 1));
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|c| *c == '*')
}

/// A config file's text with each `Include` replaced by the files it names (relative paths are
/// in `~/.ssh`, `*` in a file name matches any), at most a few levels deep.
fn read_with_includes(path: &Path, dir: &Path, depth: usize) -> String {
    let Ok(text) = std::fs::read_to_string(path) else {
        return String::new();
    };
    let mut out = String::new();
    for line in text.lines() {
        match split_line(line) {
            Some((keyword, value)) if keyword == "include" && depth < 4 => {
                for pattern in value.split_whitespace() {
                    for file in include_files(pattern, dir) {
                        out.push_str(&read_with_includes(&file, dir, depth + 1));
                        out.push('\n');
                    }
                }
            }
            _ => {
                out.push_str(line);
                out.push('\n');
            }
        }
    }
    out
}

/// The files an `Include` pattern names, sorted.
fn include_files(pattern: &str, dir: &Path) -> Vec<PathBuf> {
    let expanded = match (pattern.strip_prefix("~/"), home()) {
        (Some(rest), Some(home)) => home.join(rest),
        _ => dir.join(pattern),
    };
    let Some(name) = expanded
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
    else {
        return Vec::new();
    };
    if !name.contains(['*', '?']) {
        return vec![expanded];
    }
    let Some(parent) = expanded.parent() else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = std::fs::read_dir(parent)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .is_some_and(|n| glob(&name, &n.to_string_lossy()))
        })
        .collect();
    files.sort();
    files
}

/// `~` and the `%d` (home), `%h` (host) and `%u`/`%r` (user) tokens in a path.
fn expand_path(path: &str, home: Option<&Path>, host: &str, user: &str) -> PathBuf {
    let home_text = home
        .map(|h| h.to_string_lossy().into_owned())
        .unwrap_or_default();
    let path = path
        .replace("%d", &home_text)
        .replace("%h", host)
        .replace("%u", user)
        .replace("%r", user);
    match (path.strip_prefix('~'), home) {
        (Some(rest), Some(home)) => home.join(rest.trim_start_matches(['/', '\\'])),
        _ => PathBuf::from(path),
    }
}

/// A scratch home folder for tests, so they never read or write the real `~/.ssh`.
#[cfg(test)]
pub(crate) static TEST_HOME: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

/// The user's home folder.
pub(crate) fn home() -> Option<PathBuf> {
    #[cfg(test)]
    if let Some(home) = TEST_HOME.get() {
        return Some(home.clone());
    }
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

/// The local login name, the default remote one.
fn local_user() -> Option<String> {
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .ok()
        .filter(|u| !u.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONFIG: &str = "\
# global
ServerAliveInterval 30

Host pi raspberry*
    HostName 192.168.1.20
    User pi
    IdentityFile ~/.ssh/pi_key
    IdentityFile ~/.ssh/other

Host *.work !bastion.work
    ProxyJump me@bastion.work:2222
    Port=2200

Match host foo
    User wrong

Host *
    User fallback
    IdentityFile ~/.ssh/id_%h
";

    fn target(host: &str) -> SshTarget {
        SshTarget {
            host: host.into(),
            ..SshTarget::default()
        }
    }

    #[test]
    fn first_matching_value_wins() {
        let home = Path::new("/home/me");
        let pi = resolve_with(&target("pi"), CONFIG, Some(home));
        assert_eq!(pi.hostname, "192.168.1.20");
        assert_eq!(pi.user, "pi");
        assert_eq!(pi.port, 22);
        assert_eq!(
            pi.identity_files,
            [
                home.join(".ssh/pi_key"),
                home.join(".ssh/other"),
                home.join(".ssh/id_pi")
            ]
        );

        let work = resolve_with(&target("db.work"), CONFIG, Some(home));
        assert_eq!(work.port, 2200);
        assert_eq!(work.user, "fallback");
        assert_eq!(work.proxy_jump, ["me@bastion.work:2222"]);
        let bastion = resolve_with(&target("bastion.work"), CONFIG, Some(home));
        assert!(bastion.proxy_jump.is_empty());
        assert_eq!(bastion.port, 22);
    }

    #[test]
    fn the_profile_wins_over_the_file() {
        let t = SshTarget {
            host: "root@pi".into(),
            port: Some(2022),
            ..SshTarget::default()
        };
        let r = resolve_with(&t, CONFIG, Some(Path::new("/h")));
        assert_eq!(
            (r.user.as_str(), r.port, r.alias.as_str()),
            ("root", 2022, "pi")
        );
        assert_eq!(r.hostname, "192.168.1.20");
    }

    #[test]
    fn jump_hosts_split() {
        let j = jump_target("me@bastion:2222");
        assert_eq!(
            (j.user.as_deref(), j.host.as_str(), j.port),
            (Some("me"), "bastion", Some(2222))
        );
        assert_eq!(jump_target("bastion").port, None);
    }

    #[test]
    fn globs() {
        assert!(glob("raspberry*", "raspberrypi"));
        assert!(glob("h?st", "HOST"));
        assert!(!glob("*.work", "work"));
        assert!(host_matches("*.work !bastion.work", "db.work"));
        assert!(!host_matches("*.work !bastion.work", "bastion.work"));
    }
}
