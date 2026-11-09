//! Which shells exist on this machine and how to start them.
//!
//! macOS and Linux default to the user's login shell. Windows defaults to Git Bash (from Git
//! for Windows: a real bash with ls, mkdir, rm, grep...) when it's installed, otherwise
//! PowerShell 7, otherwise Windows PowerShell.
//!
//! [`command`] builds the [`CommandBuilder`] that [`Session::spawn`](crate::Session::spawn) runs,
//! adding the shell integration that reports the working directory (OSC 7) and marks prompts and
//! commands (OSC 133): for bash a `PROMPT_COMMAND` and `PS0` (plus `CHERE_INVOKING` for Git
//! Bash), for zsh a `ZDOTDIR` whose `.zshenv` (`zshenv.zsh`) hands back to the user's own files
//! after adding hooks, for PowerShell a script (`prompt.ps1`) run after the user's profile that
//! wraps `prompt`. [`available`] feeds the app's new-tab menu. [`vscode`] lives here too because it's
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
/// It reports the last command's exit code (OSC 133 `D`, from `$?` captured before anything
/// else runs), then the working directory (OSC 7) so restored tabs reopen where they were,
/// as a Windows path, since the app, `git -C` and restored tabs need one. `/c/Users/me` becomes
/// `C:/Users/me` without a fork; any other MSYS mount (`/tmp`, `/usr`) is asked of `pwd -W`,
/// whose subshell is the one fork, and only there. Last, it marks where the prompt starts
/// (OSC 133 `A`). Before all that, `PS1` gets OSC 133 `B` at its end, where the command line
/// starts, unless it has it (a prompt rebuilt each time loses it and gets it back). [`BASH_PS0`]
/// marks where each command's output starts.
const BASH_PROMPT_COMMAND: &str = r#"__mcr_x=$?; [[ $PS1 == *'133;B'* ]] || PS1=$PS1'\[\e]133;B\a\]'; if [[ -z $__mcr_nl ]]; then __mcr_nl=1; __mcr_r=$PS1; while [[ $__mcr_r == '\['*'\]'* ]]; do __mcr_r=${__mcr_r#*'\]'}; done; if [[ $__mcr_r == '\n'* ]]; then __mcr_ps1=$PS1; PS1=${PS1%"$__mcr_r"}${__mcr_r#'\n'}; __mcr_short=$PS1; fi; elif [[ $__mcr_nl == 1 ]]; then __mcr_nl=2; [[ -n $__mcr_ps1 && $PS1 == "$__mcr_short" ]] && PS1=$__mcr_ps1; fi
__mcr_d=$PWD; if [[ $__mcr_d =~ ^/([a-zA-Z])(/.*)?$ ]]; then __mcr_d="${BASH_REMATCH[1]^^}:${BASH_REMATCH[2]:-/}"; else __mcr_d=$(pwd -W 2>/dev/null || printf '%s' "$PWD"); fi; printf '\033]133;D;%s\007\033]7;file://%s/%s\007\033]133;A\007' "$__mcr_x" "${HOSTNAME:-localhost}" "${__mcr_d#/}""#;

/// Printed by bash after reading a command and before running it: OSC 133 `C`, where the
/// command's output starts. With `PROMPT_COMMAND`'s `D` and `A` it brackets each command.
const BASH_PS0: &str = r"\e]133;C\a";

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

/// The program's name without its folder or extension. Both `/` and `\` count as separators on
/// every platform: `Path` only knows its own, and a config can name a Windows path anywhere.
fn program_name(program: &str) -> &str {
    let file = program.rsplit(['/', '\\']).next().unwrap_or(program);
    match file.rsplit_once('.') {
        Some((stem, _)) if !stem.is_empty() => stem,
        _ => file,
    }
}

/// Whether `program` is a bash (including Git Bash), which gets macaroni's prompt integration.
pub fn is_bash(program: &str) -> bool {
    program_name(program).eq_ignore_ascii_case("bash")
}

/// Bash's prompt command outside Windows: the last command's exit code (OSC 133 `D`), the
/// working directory (OSC 7) and the prompt's start (OSC 133 `A`). Builtins only, like
/// [`BASH_PROMPT_COMMAND`], which also handles Git Bash's MSYS paths and first-prompt newline.
const BASH_PROMPT_COMMAND_UNIX: &str = r#"__mcr_x=$?; [[ $PS1 == *'133;B'* ]] || PS1=$PS1'\[\e]133;B\a\]'; printf '\033]133;D;%s\007\033]7;file://%s%s\007\033]133;A\007' "$__mcr_x" "${HOSTNAME:-localhost}" "$PWD""#;

/// The zsh integration, written to [`zsh_dir`] as `.zshenv` (see the file for how it hands
/// control back to the user's own startup files).
const ZSHENV: &str = include_str!("zshenv.zsh");

/// The PowerShell integration ([`powershell_args`]).
const PROMPT_PS1: &str = include_str!("prompt.ps1");

/// Whether `program` is PowerShell: Windows PowerShell (`powershell`) or PowerShell 7 (`pwsh`).
pub fn is_powershell(program: &str) -> bool {
    let name = program_name(program);
    name.eq_ignore_ascii_case("pwsh") || name.eq_ignore_ascii_case("powershell")
}

/// Arguments that start an interactive PowerShell with [`PROMPT_PS1`] run after the profile:
/// `-NoExit -EncodedCommand`, the script as base64 UTF-16LE, which needs no quoting on a
/// Windows command line.
fn powershell_args() -> Vec<String> {
    use base64::Engine as _;
    let utf16: Vec<u8> = PROMPT_PS1
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    vec![
        "-NoExit".into(),
        "-EncodedCommand".into(),
        base64::engine::general_purpose::STANDARD.encode(utf16),
    ]
}

/// Whether `program` is a zsh.
pub fn is_zsh(program: &str) -> bool {
    program_name(program).eq_ignore_ascii_case("zsh")
}

/// The folder zsh is pointed at (`ZDOTDIR`), with [`ZSHENV`] written in it; `None` if it can't
/// be written. It's rewritten when its text differs, so an upgrade replaces an old copy.
fn zsh_dir() -> Option<PathBuf> {
    let dir = std::env::temp_dir().join("macaroni-zsh");
    let file = dir.join(".zshenv");
    if std::fs::read_to_string(&file).ok().as_deref() != Some(ZSHENV) {
        std::fs::create_dir_all(&dir).ok()?;
        std::fs::write(&file, ZSHENV).ok()?;
    }
    Some(dir)
}

/// Adds the prompt integration for bash (`PROMPT_COMMAND`, `PS0`) and zsh (`ZDOTDIR`) to
/// `command`. Each shell reads only its own variables, so the login shell, whichever it is,
/// gets both. A user's own `PROMPT_COMMAND` (set in their `.bashrc`) replaces bash's.
fn integrate(command: &mut CommandBuilder, bash: bool, zsh: bool) {
    if bash {
        let prompt = if cfg!(windows) {
            BASH_PROMPT_COMMAND
        } else {
            BASH_PROMPT_COMMAND_UNIX
        };
        command.env("PROMPT_COMMAND", prompt);
        command.env("PS0", BASH_PS0);
    }
    if zsh && let Some(dir) = zsh_dir() {
        // The user's own ZDOTDIR (empty: unset), which the integration puts back.
        let own = std::env::var_os("ZDOTDIR").unwrap_or_default();
        command.env("MACARONI_ZDOTDIR", own);
        command.env("ZDOTDIR", dir);
    }
}

/// The command for `program` (or the platform default), with shell-specific environment.
///
/// With `program` of `None`, `args` is ignored: Windows runs [`windows_default`] with its own
/// arguments, and other platforms run the login shell (`CommandBuilder::new_default_prog`),
/// which gets both the bash and the zsh integration since it could be either.
pub fn command(program: Option<&str>, args: &[String]) -> CommandBuilder {
    let (program, args) = match program {
        Some(program) => (program.to_owned(), args.to_vec()),
        None => match windows_default() {
            Some(default) => (default.program, default.args),
            None => {
                let mut command = CommandBuilder::new_default_prog();
                integrate(&mut command, true, true);
                return command;
            }
        },
    };
    // Only a plain PowerShell: arguments of the user's own (`-NoProfile`, `-Command ...`) may
    // not start an interactive shell at all.
    let args = if is_powershell(&program) && args.is_empty() {
        powershell_args()
    } else {
        args
    };
    let mut command = CommandBuilder::new(&program);
    command.args(&args);
    if is_bash(&program) {
        // Git Bash's /etc/profile changes to $HOME unless this is set; keep the tab's directory.
        command.env("CHERE_INVOKING", "1");
    }
    integrate(&mut command, is_bash(&program), is_zsh(&program));
    command
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unix_bash_prompt_command_marks_the_prompt() {
        // Git Bash is still a bash: enough to check the Unix prompt command runs and prints.
        let Some(bash) = git_bash() else {
            return;
        };
        let out = std::process::Command::new(bash)
            .args(["-c", &format!("cd /; false; {BASH_PROMPT_COMMAND_UNIX}")])
            .output()
            .expect("run bash");
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(text.starts_with("]133;D;1]7;file://"), "{text:?}");
        assert!(text.ends_with("/]133;A"), "{text:?}");
    }

    #[test]
    fn zsh_gets_its_integration_folder() {
        assert!(is_zsh("/bin/zsh") && !is_zsh("/bin/bash"));
        let dir = zsh_dir().expect("temp folder is writable");
        let written = std::fs::read_to_string(dir.join(".zshenv")).unwrap();
        assert!(written.contains("add-zsh-hook precmd __macaroni_precmd"));
        let command = command(Some("/bin/zsh"), &[]);
        assert_eq!(command.get_env("ZDOTDIR"), Some(dir.as_os_str()));
        assert!(command.get_env("MACARONI_ZDOTDIR").is_some());
        // Bash's hook isn't added: the variable is whatever this process inherited (which is
        // macaroni's own when the tests run in a macaroni terminal).
        assert_eq!(
            command.get_env("PROMPT_COMMAND"),
            std::env::var_os("PROMPT_COMMAND").as_deref()
        );
    }

    #[test]
    fn plain_powershell_gets_its_integration() {
        assert!(is_powershell("pwsh.exe"));
        assert!(is_powershell(
            r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe"
        ));
        assert!(!is_powershell("bash.exe"));
        let args = powershell_args();
        assert_eq!(args[..2], ["-NoExit", "-EncodedCommand"]);
        use base64::Engine as _;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(&args[2])
            .unwrap();
        let units: Vec<u16> = bytes
            .chunks(2)
            .map(|p| u16::from_le_bytes([p[0], p[1]]))
            .collect();
        assert_eq!(String::from_utf16(&units).unwrap(), PROMPT_PS1);
    }

    #[test]
    fn bash_detection_by_name() {
        assert!(is_bash("C:/Program Files/Git/bin/bash.exe"));
        assert!(is_bash("/bin/bash"));
        assert!(!is_bash("pwsh.exe"));
        assert!(!is_bash("/bin/zsh"));
        // Either separator, on any platform.
        assert!(is_bash(r"C:\Program Files\Git\bin\bash.exe"));
        assert!(is_zsh("/usr/local/bin/zsh"));
        assert!(!is_bash(".bash"));
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
        assert!(
            text.starts_with("\x1b]133;D;0\x07\x1b]7;file://"),
            "{text:?}"
        );
        assert!(text.ends_with("/C:/Windows\x07\x1b]133;A\x07"), "{text:?}");
    }

    /// The first prompt loses Git Bash's leading newline and the second gets it back; a
    /// newline in the middle of a prompt is left alone. Every prompt ends in the command line
    /// mark, added once.
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
        let mark = r"\[\e]133;B\a\]";
        let git = r"\[\033]0;$PWD\007\]\n\[\033[32m\]\u@\h \w\n$ ";
        let first = git.replacen(r"\n", "", 1);
        assert_eq!(run(git), format!("{first}{mark}|{git}{mark}"));
        let two_lines = r"\u@\h\n$ ";
        assert_eq!(
            run(two_lines),
            format!("{two_lines}{mark}|{two_lines}{mark}")
        );
    }
}
