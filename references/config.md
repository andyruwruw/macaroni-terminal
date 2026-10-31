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
- `[[ssh]]` is an array of tables (`SshProfile`); each needs a `host`. The dialog edits it with `write_ssh`, which replaces the whole array, since `write_setting` only sets dotted keys.
- Font, theme and ligature changes apply live (new `Metrics` resize every view's grid); `shell` and `scrollback` apply to new terminals only.
- Themes color the terminal only. UI colors come from `ui_theme` (`Chrome`), except pane cards take the terminal background.

## Solid vs glass defaults

`ui_theme` (`UiTheme::Glass`/`Solid`) changes some defaults. Serde fills glass defaults first, so when the file is solid, `Config::parse` re-reads it as a `toml::Table` and `Style::apply_solid_defaults` gives each `SOLID_DEFAULTS` key the file doesn't set its solid value. `NumberSetting::default_for(ui)` is what the dialog shows and resets to. Two `[workspace_items]` booleans (`search`, `icon_background`) are `Option<bool>` read through `WorkspaceItemSettings::search(ui)` / `icon_background(ui)` for the same reason.

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
