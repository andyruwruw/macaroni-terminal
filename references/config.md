# Config, themes and keybindings

## Mental model

`crates/config` (`macaroni-config`) parses one TOML file plus theme files and watches them. It knows nothing about the app: actions are strings, colors are its own `Rgb`/`Rgba`. The app turns a `Config` into a keymap, `Palette`, `Chrome` and `Metrics` in `App::apply_config` (`crates/app/src/update/config.rs`).

- `config_dir()` honours `MACARONI_CONFIG_DIR`; point it at a scratch folder when testing. `config.toml` is written from `template.toml` (`TEMPLATE`) on first run; `themes/<name>.toml` shadows a built-in of the same name.
- Defaults live in code: serde defaults in `settings.rs`, `numbers!` rows in `style.rs`, `Chrome::base` and `ThemeColors::base` in the app's `theme.rs`.

## Files

- `settings.rs`: per-table structs (`FontSettings`, `WorkspaceItemSettings`, ...) and `Config::parse`, which deserializes a private `File`, applies solid defaults, range-checks, runs `Style::check` and `Colors::check`, resolves the theme and parses chords. Template tests live here.
- `style.rs`: `[style.*]` numbers (`numbers!`, `styles!`, `NumberSetting`, `Unit`, `SOLID_DEFAULTS`) and `[colors.*]` (`Colors`, `ColorValue`, `COLORS` from `colors!`, `THEME_COLORS` from `theme_colors!`).
- `theme.rs`: `Theme` (fg, bg, optional cursor/selection, 16 ansi), `BUILTIN_THEMES` embedded from `src/themes/`, `Theme::resolve`, `Theme::names`.
- `chord.rs`: chord parsing (`ctrl`, `shift`, `alt`/`option`, `cmd`/`super`/`win` plus a key; see `parse_key`).
- `edit.rs`: `write_setting(dir, key, Option<Value>)` sets or removes one dotted key with `toml_edit`, preserving comments. Only the settings dialog calls it.
- `watch.rs`: `watch(dir, on_change)` watches `.toml` files recursively with a `DEBOUNCE` (editors save in steps); the app turns callbacks into `Message::ConfigChanged`.
- App side: `keymap.rs`, `theme.rs`, `fonts.rs`, and `label.rs` (`label::fill` expands `{placeholder}` templates in `[workspace_items]`/`[tab_items]`; an empty result hides that part, so `""` is a real value).

## Invariants

- Every table is `deny_unknown_fields`, so typos are errors.
- `[colors.*]` are maps, so serde can't catch typos; `Colors::check` rejects unknown names, references to missing theme colors, and a theme color pointing at another (theme colors end the chain).
- Errors vs warnings: a parse or check error keeps the last good config and shows a banner. An unknown action (skipped by `Keymap::new`) or an uninstalled `font.family` (`fonts::resolve` falls back) is a warning; the rest applies.
- `[tab_items] new_buttons` turns the tab strip's new tab buttons on and off; it's an `item_rows` key placed on the Tabs page's "What tabs show" card.
- `[font] fallback` is a list of installed families; `fonts::set_fallback` (called by `apply_config` before measuring) resolves them, warning about missing ones, and the renderer uses them per character (see [`rendering-and-text.md`](rendering-and-text.md)).
- `[[trigger]]` (`Trigger`, `Config.triggers`): each needs a non-empty `pattern` that compiles as a Rust regex and at least one of `highlight`, `notify`, `badge`; `highlight` is a workspace color name or `#rrggbb` (an unknown name falls back to amber, so it isn't checked here). The app compiles them in `crates/app/src/triggers.rs` (`Triggers`): highlights go to the renderer, the notifying and badging patterns to every session as one `RegexSet`, and `Triggers::fire` expands the badge's `$1` and holds a notify back for `NOTIFY_COOLDOWN` per terminal. `apply_config` rebuilds them (and calls `Session::set_triggers`) only when the rules change. Not in the settings dialog.
- `[accessibility]` (`AccessibilitySettings`): `ui_scale` (0.5 to 3), `reduce_motion` and `high_contrast` (`Follow`: `"system"`, `"on"`, `"off"`; `Follow::resolve` answers `system` from the OS), `minimum_contrast` (1 to 21), `cursor_thickness` (1 to 8), `announce_output`. High contrast swaps in the built-in `HIGH_CONTRAST_THEME`. What each does: [`accessibility.md`](accessibility.md).
- `[files]` (`FileSettings`): `open` (`OpenFiles`: `"viewer"`, `"vscode"`, `"app"`) is where a Ctrl+clicked file opens, and `app` the program `"app"` runs (empty: VS Code's `code`, else the text editor). The dialog shows them on the Files page with a Choose… button (`SettingsMessage::PickApp`, the system's file dialog). What they do: [`file-tab.md`](file-tab.md).
- `[notifications] sound` and `bell_sound` take `"off"`, `"system"`, a built-in sound's name or a file path; see [`notifications.md`](notifications.md).
- `[plugins.<id>]` tables override a plugin's `on`, `agents`, `kinds`, `position`, `interval`; other keys are its settings. Plugins themselves come from the `plugins` folder next to `config.toml` (the watcher already sees their `plugin.toml`), loaded in `Config::parse` into `Config.plugins`; their problems go to `Config.plugin_problems`, which the app shows with the warnings rather than failing the config. See [`plugins.md`](plugins.md).
- `[[ssh]]` is an array of tables (`SshProfile`: `name`, `host`, `user`, `port`, `identity_file`, `proxy_jump`, `tmux`); each needs a `host`, and what it leaves out comes from `~/.ssh/config` at connect time (see [`terminal-and-pty.md`](terminal-and-pty.md)). The dialog edits it with `write_ssh`, which replaces the whole array, since `write_setting` only sets dotted keys.
- Font, theme and ligature changes apply live (new `Metrics` resize every view's grid); `shell` and `scrollback` apply to new terminals only.
- Themes color the terminal only. UI colors come from `ui_theme` (`Chrome`), except pane cards take the terminal background.

## Solid vs glass defaults

`ui_theme` (`UiTheme::Solid`, the default, or `Glass`) changes some defaults. Serde fills glass defaults first, so when the file is solid (including one that doesn't set it), `Config::parse` re-reads it as a `toml::Table` and `Style::apply_solid_defaults` gives each `SOLID_DEFAULTS` key the file doesn't set its solid value. `Config::default()` (no file at all) applies them too, so a missing file and an empty one match. The template shows solid's value for those keys with glass's in parentheses (`# key = 0  (glass: 12)`), which `template_lists_every_style_setting` checks. `NumberSetting::default_for(ui)` is what the dialog shows and resets to. Two `[workspace_items]` booleans (`search`, `icon_background`) are `Option<bool>` read through `WorkspaceItemSettings::search(ui)` / `icon_background(ui)` for the same reason.

## Colors

A `ColorValue` is `"#rrggbb"`, `"#rrggbbaa"` or the name of a theme color to follow. Each `COLORS` row has a `link` (theme color it follows while unset, may be empty) and a glass-only flag; group "Solid theme" rows are drawn only by solid. `Chrome::new` resolves: own value, else linked theme color, else the interface's base. `border` is special: while set it is copied to every `theme::BORDER_FOLLOWERS` entry not set itself; unset, it shows `card_border`. Setting `border` from the dialog also removes the followers' values (`App::write_setting` in `view/settings/mod.rs`).

## Keybindings (`crates/app/src/keymap.rs`)

`DEFAULTS_MAC` / `DEFAULTS_OTHER` are chord strings parsed like the file. User entries apply in order: each replaces any binding for its chord, and `"none"` just removes it so the key reaches the shell. `action(name)` maps names to `Action::Command(core::Command)` or app actions (`OpenSettings`, `ToggleSidebar`, `OpenConfig`, `Copy`, `Paste`, `Find`, `Zoom`). `named_bindings` lists effective chords for the dialog.

Matching is exact on modifiers. Digits match the physical key, named keys the logical key; letters and punctuation try the logical character, then the physical key's US-layout character (layouts may report a control char or shifted symbol).

## How to extend

- **Setting**: add the field to its `*Settings` struct with a serde default (and to `File`/`Config` for a new table), add a range check in `Config::parse` if needed, and add the commented line to `template.toml`. `template_parses_to_defaults` fails if the template and `Default` disagree.
- **`[style.*]` number**: add a row to the table's `numbers!` (default, min, max, step, `Unit`, label, description). That generates the field, default, range check, `Style::get` and the dialog row; place its key on a card in the app's `LOOK` table (`view/settings/list.rs`). Add `# key = default` to the template's `[style.<table>]` (with `  (solid: N)` if you also add it to `SOLID_DEFAULTS`); `template_lists_every_style_setting` enforces it. A new table also needs a `styles!` entry.
- **Color**: add a `colors!` row, a field on `Chrome` with its base value in `Chrome::glass`/`Chrome::solid`, an arm in `Chrome::color_mut`, mention the name in the template (the same test checks `COLORS`), and place it on a card in the app's `LOOK` table. A new shared color goes in `theme_colors!` and `ThemeColors::base`.
- **Built-in theme**: add `src/themes/<name>.toml` and a `BUILTIN_THEMES` entry (bump its array length).
- **Keybinding action**: add a name in `keymap::action`, a default in both `DEFAULTS_*` lists if wanted, and list it under `# Actions:` in the template; `every_documented_action_exists` checks each listed name resolves.

## Traps

- iced font names are `&'static str`. `fonts::resolve` checks the family against cosmic-text's database and interns each distinct name (leaked once) so reloads don't grow memory.
- All style values are `f32`, so ints and floats both parse.
