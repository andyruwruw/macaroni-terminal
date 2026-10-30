//! Which shells exist on this machine and how to start them.
//!
//! macOS and Linux default to the user's login shell. Windows defaults to Git Bash (from Git
//! for Windows: a real bash with ls, mkdir, rm, grep...) when it's installed, otherwise
//! PowerShell 7, otherwise Windows PowerShell.
//!
//! [`command`] builds the [`CommandBuilder`] that [`Session::spawn`](crate::Session::spawn) runs,
//! adding the bash integration (`CHERE_INVOKING`, and on Windows a `PROMPT_COMMAND` that reports
//! OSC 7). [`available`] feeds the app's new-tab menu. [`vscode`] lives here too because it's
//! the same "find a program on this machine" problem.

use std::path::{Path, PathBuf};

use portable_pty::CommandBuilder;

/// A shell the user can pick for a new tab.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellOption {
    /// Shown in the new-tab menu.
    pub name: String,
    /// The executable: a full path, or a bare name resolved through `PATH`.
    pub program: String,
    /// Arguments that make it an interactive login shell where that matters.
    pub args: Vec<String>,
}

/// Runs before every prompt. Bash builtins only: forking a process per prompt is slow under
/// MSYS on Windows.
///
/// First, Git Bash's prompt starts with a newline to set each prompt off from the output
/// above, which leaves a blank top line in a new terminal. For the first prompt only, a
/// newline that comes before anything visible (after `\[...\]` escapes) is dropped; the next
/// prompt restores it, unless the user changed `PS1` in between.
///
/// Then it reports the working directory (OSC 7) so restored tabs reopen where they were.
/// `/c/Users/me` becomes `C:/Users/me`.
const BASH_PROMPT_COMMAND: &str = r#"if [[ -z $__mcr_nl ]]; then __mcr_nl=1; __mcr_r=$PS1; while [[ $__mcr_r == '\['*'\]'* ]]; do __mcr_r=${__mcr_r#*'\]'}; done; if [[ $__mcr_r == '\n'* ]]; then __mcr_ps1=$PS1; PS1=${PS1%"$__mcr_r"}${__mcr_r#'\n'}; __mcr_short=$PS1; fi; elif [[ $__mcr_nl == 1 ]]; then __mcr_nl=2; [[ -n $__mcr_ps1 && $PS1 == "$__mcr_short" ]] && PS1=$__mcr_ps1; fi
__mcr_d=$PWD; [[ $__mcr_d =~ ^/([a-zA-Z])(/.*)?$ ]] && __mcr_d="${BASH_REMATCH[1]^^}:${BASH_REMATCH[2]:-/}"; printf '\033]7;file://%s/%s\007' "${HOSTNAME:-localhost}" "${__mcr_d#/}""#;

/// The first file called `name` in a `PATH` directory. No `PATHEXT` handling: pass the full
/// file name (`git.exe`).
fn on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|p| p.is_file())
}

/// Git for Windows' `bin\bash.exe` (the launcher that sets up the MSYS environment), found in
/// the usual install locations or next to `git.exe` on `PATH`. `C:\Windows\System32\bash.exe`
/// is WSL's launcher, not Git Bash, and is never returned.
pub fn git_bash() -> Option<PathBuf> {
    if !cfg!(windows) {
        return None;
    }
    let mut candidates = Vec::new();
    for var in ["ProgramW6432", "ProgramFiles", "ProgramFiles(x86)"] {
        if let Some(dir) = std::env::var_os(var) {
            candidates.push(PathBuf::from(dir).join("Git").join("bin").join("bash.exe"));
        }
    }
    if let Some(dir) = std::env::var_os("LOCALAPPDATA") {
        candidates.push(
            PathBuf::from(dir)
                .join("Programs")
                .join("Git")
                .join("bin")
                .join("bash.exe"),
        );
    }
    // git.exe lives in <Git>\cmd or <Git>\bin.
    if let Some(git) = on_path("git.exe")
        && let Some(root) = git.parent().and_then(Path::parent)
    {
        candidates.push(root.join("bin").join("bash.exe"));
    }
    candidates.into_iter().find(|p| p.is_file())
}

/// PowerShell 7 (`pwsh.exe`) when it's on `PATH`, else the Windows PowerShell that ships with
/// Windows.
fn powershell() -> ShellOption {
    let (name, program) = if on_path("pwsh.exe").is_some() {
        ("PowerShell", "pwsh.exe")
    } else {
        ("Windows PowerShell", "powershell.exe")
    };
    ShellOption {
        name: name.into(),
        program: program.into(),
        args: Vec::new(),
    }
}

/// The menu entry for Git Bash at `path`. `--login -i` runs `/etc/profile`, which sets up the
/// MSYS `PATH` and prompt.
fn git_bash_option(path: &Path) -> ShellOption {
    ShellOption {
        name: "Git Bash".into(),
        program: path.to_string_lossy().into_owned(),
        args: vec!["--login".into(), "-i".into()],
    }
}

/// The shell new terminals run when none is configured, on Windows. `None` elsewhere, where the
/// login shell is used.
pub fn windows_default() -> Option<ShellOption> {
    cfg!(windows).then(|| git_bash().map_or_else(powershell, |p| git_bash_option(&p)))
}

/// VS Code's `code` command: on `PATH`, else where the macOS app keeps it.
pub fn vscode() -> Option<PathBuf> {
    if cfg!(windows) {
        return on_path("code.cmd");
    }
    on_path("code").or_else(|| {
        let app = Path::new("/Applications/Visual Studio Code.app/Contents/Resources/app/bin/code");
        app.is_file().then(|| app.to_path_buf())
    })
}

/// Shells for the new-tab menu, the default first.
///
/// Probes the file system and `PATH` on every call. On macOS and Linux the login shell comes
/// from `$SHELL` (falling back to `/bin/sh`), followed by any common shells installed in the
/// usual places that aren't the same file.
pub fn available() -> Vec<ShellOption> {
    let mut shells = Vec::new();
    if cfg!(windows) {
        if let Some(bash) = git_bash() {
            shells.push(git_bash_option(&bash));
        }
        if on_path("pwsh.exe").is_some() {
            shells.push(ShellOption {
                name: "PowerShell".into(),
                program: "pwsh.exe".into(),
                args: Vec::new(),
            });
        }
        shells.push(ShellOption {
            name: "Windows PowerShell".into(),
            program: "powershell.exe".into(),
            args: Vec::new(),
        });
        shells.push(ShellOption {
            name: "Command Prompt".into(),
            program: "cmd.exe".into(),
            args: Vec::new(),
        });
    } else {
        let login = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
        let mut seen = vec![login.clone()];
        shells.push(ShellOption {
            name: Path::new(&login)
                .file_name()
                .map_or_else(|| login.clone(), |n| n.to_string_lossy().into_owned()),
            program: login,
            args: vec!["-l".into()],
        });
        for (name, program) in [
            ("zsh", "/bin/zsh"),
            ("bash", "/bin/bash"),
            ("fish", "/opt/homebrew/bin/fish"),
            ("fish", "/usr/local/bin/fish"),
        ] {
            if Path::new(program).is_file() && !seen.iter().any(|s| s == program) {
                seen.push(program.into());
                shells.push(ShellOption {
                    name: name.into(),
                    program: program.into(),
                    args: vec!["-l".into()],
                });
            }
        }
    }
    shells
}

/// Whether `program` is a bash (including Git Bash), which gets macaroni's prompt integration.
pub fn is_bash(program: &str) -> bool {
    Path::new(program)
        .file_stem()
        .is_some_and(|s| s.eq_ignore_ascii_case("bash"))
}

/// The command for `program` (or the platform default), with shell-specific environment.
///
/// With `program` of `None`, `args` is ignored: Windows runs [`windows_default`] with its own
/// arguments, and other platforms run the login shell (`CommandBuilder::new_default_prog`), which
/// gets no bash integration.
pub fn command(program: Option<&str>, args: &[String]) -> CommandBuilder {
    let (program, args) = match program {
        Some(program) => (program.to_owned(), args.to_vec()),
        None => match windows_default() {
            Some(default) => (default.program, default.args),
            None => return CommandBuilder::new_default_prog(),
        },
    };
    let mut command = CommandBuilder::new(&program);
    command.args(&args);
    if is_bash(&program) {
        // Git Bash's /etc/profile changes to $HOME unless this is set; keep the tab's directory.
        command.env("CHERE_INVOKING", "1");
        // Windows only: the prompt command exists for Git Bash (its MSYS paths and its PS1).
        // Elsewhere bash keeps the user's own PROMPT_COMMAND untouched.
        if cfg!(windows) {
            command.env("PROMPT_COMMAND", BASH_PROMPT_COMMAND);
        }
    }
    command
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bash_detection_by_name() {
        assert!(is_bash("C:/Program Files/Git/bin/bash.exe"));
        assert!(is_bash("/bin/bash"));
        assert!(!is_bash("pwsh.exe"));
        assert!(!is_bash("/bin/zsh"));
    }

    #[test]
    fn available_shells_start_with_the_default() {
        let shells = available();
        assert!(!shells.is_empty());
        if let Some(default) = windows_default() {
            assert_eq!(shells[0], default);
        }
    }

    #[test]
    #[cfg(windows)]
    fn git_bash_is_never_the_wsl_launcher() {
        if let Some(bash) = git_bash() {
            let lower = bash.to_string_lossy().to_ascii_lowercase();
            assert!(!lower.contains("system32"), "{lower}");
            assert!(lower.ends_with("bin\\bash.exe"), "{lower}");
        }
    }

    /// Runs the injected PROMPT_COMMAND in a real Git Bash and checks the OSC 7 it prints.
    #[test]
    #[cfg(windows)]
    fn prompt_command_reports_a_windows_path() {
        let Some(bash) = git_bash() else {
            return;
        };
        let out = std::process::Command::new(bash)
            .args(["-c", &format!("cd /c/Windows && {BASH_PROMPT_COMMAND}")])
            .output()
            .expect("run bash");
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(text.starts_with("\x1b]7;file://"), "{text:?}");
        assert!(text.ends_with("/C:/Windows\x07"), "{text:?}");
    }

    /// The first prompt loses Git Bash's leading newline and the second gets it back; a
    /// newline in the middle of a prompt is left alone.
    #[test]
    #[cfg(windows)]
    fn first_prompt_skips_the_leading_newline() {
        let Some(bash) = git_bash() else {
            return;
        };
        let run = |ps1: &str| {
            let script = format!(
                "PS1='{ps1}'; {BASH_PROMPT_COMMAND} >/dev/null; printf '%s|' \"$PS1\"; \
                 {BASH_PROMPT_COMMAND} >/dev/null; printf '%s' \"$PS1\""
            );
            let out = std::process::Command::new(&bash)
                .args(["-c", &script])
                .output()
                .expect("run bash");
            String::from_utf8_lossy(&out.stdout).into_owned()
        };
        let git = r"\[\033]0;$PWD\007\]\n\[\033[32m\]\u@\h \w\n$ ";
        let first = git.replacen(r"\n", "", 1);
        assert_eq!(run(git), format!("{first}|{git}"));
        let two_lines = r"\u@\h\n$ ";
        assert_eq!(run(two_lines), format!("{two_lines}|{two_lines}"));
    }
}
