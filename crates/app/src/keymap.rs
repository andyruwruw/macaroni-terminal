//! Shortcuts: platform defaults plus the user's `[keybindings]` overrides from the config file.
//!
//! Windows and Linux default to Ctrl+Shift so that plain Ctrl combinations keep reaching the
//! shell; macOS uses Cmd, which shells never see.
//!
//! [`Keymap::lookup`] is asked by `TerminalView` before it encodes a key, by browsers (through a
//! shared copy, `App.shared_keymap`) while a page has focus, and by `App::unhandled_key` for
//! keys nothing else took. A match becomes an [`Action`], which `App::perform` carries out.
//! [`action`] is the one table from config names to actions; the settings dialog lists
//! [`named_bindings`].

use iced::keyboard::key::{Code, Named, Physical};
use iced::keyboard::{Key, Modifiers};
use macaroni_config::{Chord, ChordKey, NamedKey};
use macaroni_core::{Command, Direction};

/// What a shortcut does.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// A model command, applied like any other.
    Command(Command),
    /// Open `config.toml` in the system's editor.
    OpenConfig,
    /// Open the settings dialog.
    OpenSettings,
    /// Copy the focused terminal's selection.
    Copy,
    /// Paste the clipboard into the focused terminal.
    Paste,
    /// Open the search bar on the focused pane.
    Find,
    /// Fold the sidebar away, or bring it back.
    ToggleSidebar,
    /// Open the command palette.
    CommandPalette,
    /// Open the palette listing the tabs waiting for the user.
    ShowWaiting,
    /// Open the new worktree dialog for the active workspace.
    NewWorktree,
    /// Scroll the focused terminal to the previous (`true`) or next prompt its shell marked.
    Prompt(bool),
    /// Copy the output of the focused terminal's last command its shell marked.
    CopyOutput,
    /// Open or close the agent dashboard.
    AgentDashboard,
    /// Show a letter over each pane; the next key focuses that pane.
    JumpToPane,
    /// Bring back the most recently closed tab.
    ReopenTab,
    /// Save the focused terminal's text to a file.
    SaveScrollback,
    /// Search the scrollback of every terminal (`true`) or the active workspace's.
    SearchScrollback(bool),
    /// The terminal font size, for this run only (Ctrl+scroll does this too).
    Zoom(Zoom),
}

/// Which way [`Action::Zoom`] changes the font size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Zoom {
    /// A point bigger.
    In,
    /// A point smaller.
    Out,
    /// Back to the configured size.
    Reset,
}

/// macOS defaults: chord and action name.
const DEFAULTS_MAC: &[(&str, &str)] = &[
    ("cmd+t", "new_tab"),
    ("cmd+w", "close_tab"),
    ("ctrl+tab", "next_tab"),
    ("ctrl+shift+tab", "previous_tab"),
    ("cmd+d", "split_right"),
    ("cmd+shift+d", "split_down"),
    ("cmd+alt+left", "focus_left"),
    ("cmd+alt+right", "focus_right"),
    ("cmd+alt+up", "focus_up"),
    ("cmd+alt+down", "focus_down"),
    ("cmd+alt+shift+left", "swap_left"),
    ("cmd+alt+shift+right", "swap_right"),
    ("cmd+alt+shift+up", "swap_up"),
    ("cmd+alt+shift+down", "swap_down"),
    ("cmd+shift+enter", "maximize_pane"),
    ("cmd+n", "new_workspace"),
    ("cmd+shift+]", "next_workspace"),
    ("cmd+shift+[", "previous_workspace"),
    ("cmd+1", "workspace_1"),
    ("cmd+2", "workspace_2"),
    ("cmd+3", "workspace_3"),
    ("cmd+4", "workspace_4"),
    ("cmd+5", "workspace_5"),
    ("cmd+6", "workspace_6"),
    ("cmd+7", "workspace_7"),
    ("cmd+8", "workspace_8"),
    ("cmd+9", "workspace_9"),
    ("cmd+,", "open_settings"),
    ("cmd+b", "toggle_sidebar"),
    ("cmd+shift+p", "command_palette"),
    ("cmd+shift+i", "show_waiting"),
    ("cmd+shift+u", "next_waiting"),
    ("cmd+up", "previous_prompt"),
    ("cmd+down", "next_prompt"),
    ("cmd+shift+o", "copy_output"),
    ("cmd+shift+a", "agent_dashboard"),
    ("cmd+shift+j", "jump_to_pane"),
    ("cmd+shift+r", "reopen_tab"),
    ("cmd+alt+f", "search_all"),
    ("cmd+shift+,", "open_config"),
    ("cmd+c", "copy"),
    ("cmd+v", "paste"),
    ("cmd+f", "find"),
    ("cmd+=", "zoom_in"),
    ("cmd+-", "zoom_out"),
    ("cmd+0", "reset_zoom"),
];

/// Windows and Linux defaults: chord and action name.
const DEFAULTS_OTHER: &[(&str, &str)] = &[
    ("ctrl+shift+t", "new_tab"),
    ("ctrl+shift+w", "close_tab"),
    ("ctrl+tab", "next_tab"),
    ("ctrl+shift+tab", "previous_tab"),
    ("ctrl+shift+d", "split_right"),
    ("ctrl+shift+e", "split_down"),
    ("ctrl+alt+left", "focus_left"),
    ("ctrl+alt+right", "focus_right"),
    ("ctrl+alt+up", "focus_up"),
    ("ctrl+alt+down", "focus_down"),
    ("ctrl+alt+shift+left", "swap_left"),
    ("ctrl+alt+shift+right", "swap_right"),
    ("ctrl+alt+shift+up", "swap_up"),
    ("ctrl+alt+shift+down", "swap_down"),
    ("ctrl+shift+enter", "maximize_pane"),
    ("ctrl+shift+n", "new_workspace"),
    ("ctrl+shift+pagedown", "next_workspace"),
    ("ctrl+shift+pageup", "previous_workspace"),
    ("ctrl+shift+1", "workspace_1"),
    ("ctrl+shift+2", "workspace_2"),
    ("ctrl+shift+3", "workspace_3"),
    ("ctrl+shift+4", "workspace_4"),
    ("ctrl+shift+5", "workspace_5"),
    ("ctrl+shift+6", "workspace_6"),
    ("ctrl+shift+7", "workspace_7"),
    ("ctrl+shift+8", "workspace_8"),
    ("ctrl+shift+9", "workspace_9"),
    ("ctrl+,", "open_settings"),
    ("ctrl+b", "toggle_sidebar"),
    ("ctrl+shift+p", "command_palette"),
    ("ctrl+shift+i", "show_waiting"),
    ("ctrl+shift+u", "next_waiting"),
    ("ctrl+shift+up", "previous_prompt"),
    ("ctrl+shift+down", "next_prompt"),
    ("ctrl+shift+o", "copy_output"),
    ("ctrl+shift+a", "agent_dashboard"),
    ("ctrl+shift+j", "jump_to_pane"),
    ("ctrl+shift+r", "reopen_tab"),
    ("ctrl+shift+g", "search_all"),
    ("ctrl+shift+,", "open_config"),
    ("ctrl+shift+c", "copy"),
    ("ctrl+shift+v", "paste"),
    ("ctrl+v", "paste"),
    ("shift+insert", "paste"),
    ("ctrl+insert", "copy"),
    ("ctrl+shift+f", "find"),
    ("ctrl+f", "find"),
    ("ctrl+=", "zoom_in"),
    ("ctrl+-", "zoom_out"),
    ("ctrl+0", "reset_zoom"),
];

/// The action a config name (`new_tab`, `workspace_3`...) stands for; `None` for an unknown
/// name. `none` isn't an action: callers treat it as unbinding.
pub fn action(name: &str) -> Option<Action> {
    let command = match name {
        "open_config" => return Some(Action::OpenConfig),
        "open_settings" => return Some(Action::OpenSettings),
        "toggle_sidebar" => return Some(Action::ToggleSidebar),
        "command_palette" => return Some(Action::CommandPalette),
        "show_waiting" => return Some(Action::ShowWaiting),
        "new_worktree" => return Some(Action::NewWorktree),
        "previous_prompt" => return Some(Action::Prompt(true)),
        "next_prompt" => return Some(Action::Prompt(false)),
        "copy_output" => return Some(Action::CopyOutput),
        "agent_dashboard" => return Some(Action::AgentDashboard),
        "jump_to_pane" => return Some(Action::JumpToPane),
        "reopen_tab" => return Some(Action::ReopenTab),
        "save_scrollback" => return Some(Action::SaveScrollback),
        "search_workspace" => return Some(Action::SearchScrollback(false)),
        "search_all" => return Some(Action::SearchScrollback(true)),
        "copy" => return Some(Action::Copy),
        "paste" => return Some(Action::Paste),
        "find" => return Some(Action::Find),
        "zoom_in" => return Some(Action::Zoom(Zoom::In)),
        "zoom_out" => return Some(Action::Zoom(Zoom::Out)),
        "reset_zoom" => return Some(Action::Zoom(Zoom::Reset)),
        "new_tab" => Command::NewTab,
        "close_tab" => Command::CloseActiveTab,
        "next_tab" => Command::NextTab,
        "previous_tab" => Command::PreviousTab,
        "split_right" => Command::SplitPane(Direction::Right),
        "split_down" => Command::SplitPane(Direction::Down),
        "split_left" => Command::SplitPane(Direction::Left),
        "split_up" => Command::SplitPane(Direction::Up),
        "close_pane" => Command::ClosePane,
        "focus_left" => Command::FocusDirection(Direction::Left),
        "focus_right" => Command::FocusDirection(Direction::Right),
        "focus_up" => Command::FocusDirection(Direction::Up),
        "focus_down" => Command::FocusDirection(Direction::Down),
        "maximize_pane" => Command::ToggleMaximize,
        "equalize_panes" => Command::EqualizePanes,
        "rotate_panes" => Command::RotatePanes,
        "swap_left" => Command::SwapPane(Direction::Left),
        "swap_right" => Command::SwapPane(Direction::Right),
        "swap_up" => Command::SwapPane(Direction::Up),
        "swap_down" => Command::SwapPane(Direction::Down),
        "next_waiting" => Command::NextWaiting,
        "new_workspace" => Command::NewWorkspace,
        "close_workspace" => Command::CloseActiveWorkspace,
        "next_workspace" => Command::NextWorkspace,
        "previous_workspace" => Command::PreviousWorkspace,
        other => {
            let n: usize = other.strip_prefix("workspace_")?.parse().ok()?;
            if !(1..=9).contains(&n) {
                return None;
            }
            Command::SelectWorkspaceIndex(n - 1)
        }
    };
    Some(Action::Command(command))
}

/// Every action's config name, for the command palette: [`action`]'s names in the order the
/// palette lists them, with `workspace_1`..`workspace_9` left out (it lists the workspaces
/// themselves).
pub const NAMES: &[&str] = &[
    "new_tab",
    "close_tab",
    "reopen_tab",
    "next_tab",
    "previous_tab",
    "split_right",
    "split_down",
    "split_left",
    "split_up",
    "close_pane",
    "maximize_pane",
    "equalize_panes",
    "rotate_panes",
    "swap_left",
    "swap_right",
    "swap_up",
    "swap_down",
    "jump_to_pane",
    "show_waiting",
    "agent_dashboard",
    "next_waiting",
    "focus_left",
    "focus_right",
    "focus_up",
    "focus_down",
    "new_workspace",
    "new_worktree",
    "close_workspace",
    "next_workspace",
    "previous_workspace",
    "toggle_sidebar",
    "find",
    "search_workspace",
    "search_all",
    "previous_prompt",
    "next_prompt",
    "copy_output",
    "save_scrollback",
    "copy",
    "paste",
    "zoom_in",
    "zoom_out",
    "reset_zoom",
    "open_settings",
    "open_config",
];

/// `new_tab` as "New tab", `workspace_3` as "Workspace 3".
pub fn label(name: &str) -> String {
    let words = name.replace('_', " ");
    let mut chars = words.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

/// A chord as it's shown, e.g. Ctrl+Shift+T.
pub fn chord_label(chord: &Chord) -> String {
    let mut parts: Vec<String> = Vec::new();
    let logo = if cfg!(target_os = "macos") {
        "Cmd"
    } else {
        "Win"
    };
    for (on, name) in [
        (chord.mods.ctrl, "Ctrl"),
        (
            chord.mods.alt,
            if cfg!(target_os = "macos") {
                "Option"
            } else {
                "Alt"
            },
        ),
        (chord.mods.shift, "Shift"),
        (chord.mods.logo, logo),
    ] {
        if on {
            parts.push(name.to_owned());
        }
    }
    parts.push(match chord.key {
        ChordKey::Char(c) => c.to_uppercase().to_string(),
        ChordKey::Digit(d) => d.to_string(),
        ChordKey::Named(NamedKey::F(n)) => format!("F{n}"),
        ChordKey::Named(NamedKey::PageUp) => "PageUp".to_owned(),
        ChordKey::Named(NamedKey::PageDown) => "PageDown".to_owned(),
        ChordKey::Named(named) => format!("{named:?}"),
    });
    parts.join("+")
}

/// The shortcuts in effect.
#[derive(Clone)]
pub struct Keymap {
    /// Each chord and its action, at most one per chord; the first match wins.
    bindings: Vec<(Chord, Action)>,
}

/// This platform's default bindings.
fn defaults() -> &'static [(&'static str, &'static str)] {
    if cfg!(target_os = "macos") {
        DEFAULTS_MAC
    } else {
        DEFAULTS_OTHER
    }
}

/// Every chord in effect and the name of its action, defaults first, for listing them.
/// Unknown actions and `none` are left out, as [`Keymap::new`] leaves them out.
///
/// # Panics
///
/// If a built-in default chord doesn't parse (a bug caught by the tests).
pub fn named_bindings(overrides: &[(Chord, String)]) -> Vec<(Chord, String)> {
    let mut bindings: Vec<(Chord, String)> = defaults()
        .iter()
        .map(|(chord, name)| {
            (
                chord.parse().expect("default chords parse"),
                name.to_string(),
            )
        })
        .collect();
    for (chord, name) in overrides {
        bindings.retain(|(c, _)| c != chord);
        if name != "none" && action(name).is_some() {
            bindings.push((*chord, name.clone()));
        }
    }
    bindings
}

impl Keymap {
    /// Platform defaults with `overrides` applied in order: each override replaces any binding
    /// of its chord, and `none` just removes it. Returns problems (unknown actions) alongside a
    /// keymap that skips them.
    ///
    /// # Panics
    ///
    /// If a built-in default doesn't parse or names no action (a bug caught by the tests).
    pub fn new(overrides: &[(Chord, String)]) -> (Self, Vec<String>) {
        let defaults = defaults();
        let mut bindings: Vec<(Chord, Action)> = defaults
            .iter()
            .map(|(chord, name)| {
                (
                    chord.parse().expect("default chords parse"),
                    action(name).expect("default actions exist"),
                )
            })
            .collect();

        let mut errors = Vec::new();
        for (chord, name) in overrides {
            bindings.retain(|(c, _)| c != chord);
            if name == "none" {
                continue;
            }
            match action(name) {
                Some(action) => bindings.push((*chord, action)),
                None => errors.push(format!("keybindings: unknown action {name:?}")),
            }
        }
        (Self { bindings }, errors)
    }

    /// The action bound to a key press, if any. Modifiers must match exactly. Digits match by
    /// physical key (Shift+1 types `!`), named keys by name. Letters and punctuation match the
    /// character the layout produced first; only if nothing matches is the physical key (US
    /// layout) tried, for layouts that report something else under Ctrl or Shift.
    pub fn lookup(&self, key: &Key, physical: Physical, mods: Modifiers) -> Option<Action> {
        let logical = logical_char(key);
        let physical_char = physical_char(physical);
        let candidates = self.bindings.iter().filter(|(c, _)| mods_match(c, mods));

        let mut fallback = None;
        for (chord, action) in candidates {
            let hit = match chord.key {
                ChordKey::Digit(d) => physical_digit(physical) == Some(d),
                ChordKey::Named(n) => named(key) == Some(n),
                ChordKey::Char(c) => {
                    if logical == Some(c) {
                        true
                    } else {
                        if fallback.is_none() && physical_char == Some(c) {
                            fallback = Some(action.clone());
                        }
                        false
                    }
                }
            };
            if hit {
                return Some(action.clone());
            }
        }
        fallback
    }
}

/// Whether a press's modifiers are exactly the chord's.
fn mods_match(chord: &Chord, mods: Modifiers) -> bool {
    chord.mods.ctrl == mods.control()
        && chord.mods.shift == mods.shift()
        && chord.mods.alt == mods.alt()
        && chord.mods.logo == mods.logo()
}

/// The lowercased first character a key produced, if it's printable ASCII. Control
/// characters (what some layouts report under Ctrl) don't count.
fn logical_char(key: &Key) -> Option<char> {
    match key {
        Key::Character(s) => s
            .chars()
            .next()
            .filter(|c| c.is_ascii_graphic())
            .map(|c| c.to_ascii_lowercase()),
        _ => None,
    }
}

/// A named key as a chord's [`NamedKey`], if chords can name it.
fn named(key: &Key) -> Option<NamedKey> {
    let Key::Named(named) = key else {
        return None;
    };
    Some(match named {
        Named::Tab => NamedKey::Tab,
        Named::Enter => NamedKey::Enter,
        Named::Escape => NamedKey::Escape,
        Named::Space => NamedKey::Space,
        Named::Backspace => NamedKey::Backspace,
        Named::Delete => NamedKey::Delete,
        Named::Insert => NamedKey::Insert,
        Named::Home => NamedKey::Home,
        Named::End => NamedKey::End,
        Named::PageUp => NamedKey::PageUp,
        Named::PageDown => NamedKey::PageDown,
        Named::ArrowLeft => NamedKey::Left,
        Named::ArrowRight => NamedKey::Right,
        Named::ArrowUp => NamedKey::Up,
        Named::ArrowDown => NamedKey::Down,
        Named::F1 => NamedKey::F(1),
        Named::F2 => NamedKey::F(2),
        Named::F3 => NamedKey::F(3),
        Named::F4 => NamedKey::F(4),
        Named::F5 => NamedKey::F(5),
        Named::F6 => NamedKey::F(6),
        Named::F7 => NamedKey::F(7),
        Named::F8 => NamedKey::F(8),
        Named::F9 => NamedKey::F(9),
        Named::F10 => NamedKey::F(10),
        Named::F11 => NamedKey::F(11),
        Named::F12 => NamedKey::F(12),
        _ => return None,
    })
}

/// The digit on a top-row number key, whatever the layout or Shift produced.
fn physical_digit(physical: Physical) -> Option<u8> {
    let Physical::Code(code) = physical else {
        return None;
    };
    Some(match code {
        Code::Digit0 => 0,
        Code::Digit1 => 1,
        Code::Digit2 => 2,
        Code::Digit3 => 3,
        Code::Digit4 => 4,
        Code::Digit5 => 5,
        Code::Digit6 => 6,
        Code::Digit7 => 7,
        Code::Digit8 => 8,
        Code::Digit9 => 9,
        _ => return None,
    })
}

/// The unshifted character a physical key produces on a US layout.
fn physical_char(physical: Physical) -> Option<char> {
    let Physical::Code(code) = physical else {
        return None;
    };
    const LETTERS: [(Code, char); 26] = [
        (Code::KeyA, 'a'),
        (Code::KeyB, 'b'),
        (Code::KeyC, 'c'),
        (Code::KeyD, 'd'),
        (Code::KeyE, 'e'),
        (Code::KeyF, 'f'),
        (Code::KeyG, 'g'),
        (Code::KeyH, 'h'),
        (Code::KeyI, 'i'),
        (Code::KeyJ, 'j'),
        (Code::KeyK, 'k'),
        (Code::KeyL, 'l'),
        (Code::KeyM, 'm'),
        (Code::KeyN, 'n'),
        (Code::KeyO, 'o'),
        (Code::KeyP, 'p'),
        (Code::KeyQ, 'q'),
        (Code::KeyR, 'r'),
        (Code::KeyS, 's'),
        (Code::KeyT, 't'),
        (Code::KeyU, 'u'),
        (Code::KeyV, 'v'),
        (Code::KeyW, 'w'),
        (Code::KeyX, 'x'),
        (Code::KeyY, 'y'),
        (Code::KeyZ, 'z'),
    ];
    if let Some((_, c)) = LETTERS.iter().find(|(k, _)| *k == code) {
        return Some(*c);
    }
    Some(match code {
        Code::Comma => ',',
        Code::Period => '.',
        Code::Slash => '/',
        Code::Semicolon => ';',
        Code::Quote => '\'',
        Code::BracketLeft => '[',
        Code::BracketRight => ']',
        Code::Backslash => '\\',
        Code::Minus => '-',
        Code::Equal => '=',
        Code::Backquote => '`',
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAC: bool = cfg!(target_os = "macos");

    fn primary() -> Modifiers {
        if MAC {
            Modifiers::LOGO
        } else {
            Modifiers::CTRL | Modifiers::SHIFT
        }
    }

    fn press(keymap: &Keymap, key: Key, code: Code, mods: Modifiers) -> Option<Action> {
        keymap.lookup(&key, Physical::Code(code), mods)
    }

    fn cmd(command: Command) -> Option<Action> {
        Some(Action::Command(command))
    }

    #[test]
    fn defaults() {
        let (keymap, errors) = Keymap::new(&[]);
        assert!(errors.is_empty());
        assert_eq!(
            press(&keymap, Key::Character("T".into()), Code::KeyT, primary()),
            cmd(Command::NewTab)
        );
        assert_eq!(
            press(&keymap, Key::Character("!".into()), Code::Digit1, primary()),
            cmd(Command::SelectWorkspaceIndex(0))
        );
        assert_eq!(
            press(
                &keymap,
                Key::Character("\u{14}".into()),
                Code::KeyT,
                primary()
            ),
            cmd(Command::NewTab)
        );
        assert_eq!(
            press(&keymap, Key::Named(Named::Tab), Code::Tab, Modifiers::CTRL),
            cmd(Command::NextTab)
        );
    }

    #[test]
    fn plain_ctrl_reaches_the_shell() {
        let (keymap, _) = Keymap::new(&[]);
        assert_eq!(
            press(
                &keymap,
                Key::Character("t".into()),
                Code::KeyT,
                Modifiers::CTRL
            ),
            None
        );
        assert_eq!(
            press(&keymap, Key::Character("x".into()), Code::KeyX, primary()),
            None
        );
        assert_eq!(
            press(&keymap, Key::Character("c".into()), Code::KeyC, primary()),
            Some(Action::Copy)
        );
    }

    #[test]
    fn overrides_rebind_and_unbind() {
        let new_tab_default: Chord = if MAC { "cmd+t" } else { "ctrl+shift+t" }.parse().unwrap();
        let overrides = vec![
            (new_tab_default, "none".to_owned()),
            ("alt+t".parse().unwrap(), "new_tab".to_owned()),
            ("alt+x".parse().unwrap(), "explode".to_owned()),
        ];
        let (keymap, errors) = Keymap::new(&overrides);
        assert_eq!(errors, vec!["keybindings: unknown action \"explode\""]);
        assert_eq!(
            press(&keymap, Key::Character("T".into()), Code::KeyT, primary()),
            None
        );
        assert_eq!(
            press(
                &keymap,
                Key::Character("t".into()),
                Code::KeyT,
                Modifiers::ALT
            ),
            cmd(Command::NewTab)
        );
    }

    #[test]
    fn logical_character_wins_over_physical_position() {
        // On AZERTY the key in QWERTY's Q position types "a".
        let overrides = vec![
            ("alt+a".parse().unwrap(), "new_tab".to_owned()),
            ("alt+q".parse().unwrap(), "close_tab".to_owned()),
        ];
        let (keymap, _) = Keymap::new(&overrides);
        assert_eq!(
            press(
                &keymap,
                Key::Character("a".into()),
                Code::KeyQ,
                Modifiers::ALT
            ),
            cmd(Command::NewTab)
        );
    }

    #[test]
    fn every_documented_action_exists() {
        let actions: String = macaroni_config::TEMPLATE
            .lines()
            .map(str::trim)
            .skip_while(|l| !l.starts_with("# Actions:"))
            .take_while(|l| *l != "#")
            .collect::<Vec<_>>()
            .join(" ");
        let names: Vec<&str> = actions
            .trim_start_matches("# Actions:")
            .split(|c: char| c == ',' || c.is_whitespace() || c == '#' || c == '.')
            .filter(|s| !s.is_empty())
            .collect();
        assert!(names.len() >= 20, "{names:?}");
        for name in names {
            assert!(action(name).is_some(), "{name}");
        }
    }
}
