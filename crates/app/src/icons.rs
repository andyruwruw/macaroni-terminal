//! UI icons: Lucide (ISC, `assets/icons/LICENSE`), drawn at a 1.5 stroke, plus the logo. All are
//! single-color SVGs, tinted when drawn.
//!
//! The view draws them with [`icon`]. Each SVG is compiled in and decoded once (`handles!`
//! builds a lazily initialized handle per icon). iced's svg tint ignores the color's alpha, so
//! callers fade icons with `Svg::opacity` instead. To add one: download it from
//! `lucide-static`, thin its stroke to 1.5, add a variant to [`Icon`] and a line to
//! `handles!`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex, OnceLock};

use iced::widget::{Svg, image, svg};
use iced::{Color, Element, Length};

/// Every icon the UI draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    /// The macaroni logo in the title bar (`macaroni.svg`).
    Logo,
    /// A terminal tab, and the default workspace icon.
    Terminal,
    /// A browser tab.
    Globe,
    /// Close buttons (an x).
    Close,
    /// New tab and new workspace buttons, and the settings steppers' +.
    Plus,
    /// An expanded group's chevron.
    ChevronDown,
    /// The settings button (a gear).
    Settings,
    /// The browser's back button.
    Back,
    /// The browser's forward button, and Split right in menus.
    Forward,
    /// The browser's reload button.
    Reload,
    /// The browser's DevTools button.
    DevTools,
    /// Move up in menus, and the terminal search's previous match.
    Up,
    /// Move down and Split down in menus, and the terminal search's next match.
    Down,
    /// A waiting notification.
    Bell,
    /// A coding agent, tinted by its state.
    Sparkles,
    /// The window's minimize button, and the settings steppers' -.
    Minimize,
    /// The window's maximize button.
    Maximize,
    /// The window's restore button, while maximized.
    Restore,
    /// Rename, in menus.
    Pencil,
    /// Move, in menus.
    Move,
    /// Muted tabs and workspaces, and Mute in menus.
    BellOff,
    /// Pinned workspaces, and Pin in menus.
    Pin,
    /// Unpin, in menus.
    PinOff,
    /// A collapsed group's chevron, and menu entries that open a submenu.
    ChevronRight,
    /// Copy Tab ID and Copy Workspace ID, in menus.
    Copy,
    /// A workspace icon, and Open in VS Code.
    Code,
    /// A workspace icon (see [`WORKSPACE_ICONS`]).
    Rocket,
    /// A workspace icon (see [`WORKSPACE_ICONS`]).
    Flask,
    /// A workspace icon (see [`WORKSPACE_ICONS`]).
    Database,
    /// A workspace icon (see [`WORKSPACE_ICONS`]).
    Server,
    /// A workspace icon (see [`WORKSPACE_ICONS`]).
    Bug,
    /// A workspace icon (see [`WORKSPACE_ICONS`]).
    Book,
    /// A workspace icon (see [`WORKSPACE_ICONS`]).
    Briefcase,
    /// A workspace icon (see [`WORKSPACE_ICONS`]).
    Folder,
    /// A workspace icon (see [`WORKSPACE_ICONS`]).
    House,
    /// A workspace icon (see [`WORKSPACE_ICONS`]).
    Star,
    /// A workspace icon (see [`WORKSPACE_ICONS`]).
    Heart,
    /// A workspace icon (see [`WORKSPACE_ICONS`]).
    Zap,
    /// A workspace icon (see [`WORKSPACE_ICONS`]).
    Cpu,
    /// A workspace icon (see [`WORKSPACE_ICONS`]).
    Cloud,
    /// A workspace icon (see [`WORKSPACE_ICONS`]).
    Palette,
    /// A workspace icon (see [`WORKSPACE_ICONS`]).
    Music,
    /// A workspace icon (see [`WORKSPACE_ICONS`]).
    Gamepad,
    /// A workspace icon (see [`WORKSPACE_ICONS`]).
    Wrench,
    /// A workspace icon (see [`WORKSPACE_ICONS`]).
    Shield,
    /// A workspace icon (see [`WORKSPACE_ICONS`]).
    GitBranch,
    /// A workspace icon (see [`WORKSPACE_ICONS`]).
    Coffee,
    /// New group, in menus.
    FolderPlus,
    /// Set Active Folder, in menus.
    FolderPen,
    /// Combine Workspaces, in menus.
    Combine,
    /// The settings dialog's Theme page.
    SwatchBook,
    /// Remove from group, in menus.
    FolderMinus,
    /// No color or icon, in pickers.
    Ban,
    /// The settings dialog's search field.
    Search,
    /// The settings dialog's Keyboard Shortcuts section.
    Keyboard,
    /// The settings dialog's General section.
    Sliders,
    /// The settings dialog's Sidebar page, and the button that folds the sidebar.
    PanelLeft,
    /// Open config.toml, in the settings dialog.
    ExternalLink,
    /// The settings dialog's Agents section.
    Bot,
    /// The settings dialog's Window page.
    AppWindow,
    /// The settings dialog's Tabs page.
    PanelTop,
    /// The settings dialog's Panes page.
    Columns,
    /// The settings dialog's Menus & Controls page.
    Menu,
}

/// Defines `handle`, mapping each [`Icon`] to its file in `assets/icons/`: one lazily decoded
/// static handle per icon, so every draw clones a shared handle (and iced's svg cache keys on
/// it) instead of parsing the SVG again.
macro_rules! handles {
    ($($icon:ident => $file:literal),* $(,)?) => {
        /// The shared SVG handle for `icon`.
        fn handle(icon: Icon) -> svg::Handle {
            $(
                #[allow(non_upper_case_globals)]
                static $icon: LazyLock<svg::Handle> = LazyLock::new(|| {
                    svg::Handle::from_memory(include_bytes!(concat!("../assets/icons/", $file)))
                });
            )*
            match icon {
                $(Icon::$icon => $icon.clone(),)*
            }
        }
    };
}

handles! {
    Logo => "macaroni.svg",
    Terminal => "square-terminal.svg",
    Globe => "globe.svg",
    Close => "x.svg",
    Plus => "plus.svg",
    ChevronDown => "chevron-down.svg",
    Settings => "settings.svg",
    Back => "arrow-left.svg",
    Forward => "arrow-right.svg",
    Reload => "rotate-cw.svg",
    DevTools => "code-xml.svg",
    Up => "arrow-up.svg",
    Down => "arrow-down.svg",
    Bell => "bell.svg",
    Sparkles => "sparkles.svg",
    Minimize => "minus.svg",
    Maximize => "square.svg",
    Restore => "copy.svg",
    Pencil => "pencil.svg",
    Move => "arrow-right-left.svg",
    BellOff => "bell-off.svg",
    Pin => "pin.svg",
    PinOff => "pin-off.svg",
    ChevronRight => "chevron-right.svg",
    Copy => "copy.svg",
    Code => "code.svg",
    Rocket => "rocket.svg",
    Flask => "flask-conical.svg",
    Database => "database.svg",
    Server => "server.svg",
    Bug => "bug.svg",
    Book => "book-open.svg",
    Briefcase => "briefcase.svg",
    Folder => "folder.svg",
    House => "house.svg",
    Star => "star.svg",
    Heart => "heart.svg",
    Zap => "zap.svg",
    Cpu => "cpu.svg",
    Cloud => "cloud.svg",
    Palette => "palette.svg",
    Music => "music.svg",
    Gamepad => "gamepad-2.svg",
    Wrench => "wrench.svg",
    Shield => "shield.svg",
    GitBranch => "git-branch.svg",
    Coffee => "coffee.svg",
    FolderPlus => "folder-plus.svg",
    FolderPen => "folder-pen.svg",
    Combine => "combine.svg",
    SwatchBook => "swatch-book.svg",
    FolderMinus => "folder-minus.svg",
    Ban => "ban.svg",
    Search => "search.svg",
    Bot => "bot.svg",
    Keyboard => "keyboard.svg",
    Sliders => "sliders-horizontal.svg",
    PanelLeft => "panel-left.svg",
    ExternalLink => "external-link.svg",
    AppWindow => "app-window.svg",
    PanelTop => "panel-top.svg",
    Columns => "columns-2.svg",
    Menu => "menu.svg",
}

/// The icons a workspace can have, by the names saved with it.
pub const WORKSPACE_ICONS: [(&str, Icon); 24] = [
    ("terminal", Icon::Terminal),
    ("code", Icon::Code),
    ("globe", Icon::Globe),
    ("rocket", Icon::Rocket),
    ("flask", Icon::Flask),
    ("database", Icon::Database),
    ("server", Icon::Server),
    ("bug", Icon::Bug),
    ("book", Icon::Book),
    ("briefcase", Icon::Briefcase),
    ("folder", Icon::Folder),
    ("house", Icon::House),
    ("star", Icon::Star),
    ("heart", Icon::Heart),
    ("zap", Icon::Zap),
    ("cpu", Icon::Cpu),
    ("cloud", Icon::Cloud),
    ("palette", Icon::Palette),
    ("music", Icon::Music),
    ("gamepad", Icon::Gamepad),
    ("wrench", Icon::Wrench),
    ("shield", Icon::Shield),
    ("git-branch", Icon::GitBranch),
    ("coffee", Icon::Coffee),
];

/// A workspace's icon: its chosen one (see [`named_glyph`]), else the terminal.
pub fn workspace_icon(name: Option<&str>) -> Glyph {
    named_glyph(name).unwrap_or(Glyph::Icon(Icon::Terminal))
}

/// What an item (a workspace or tab) shows as its icon: one of the UI's own icons, an SVG (from
/// the library, or a file the user added) tinted like them, or a picture drawn as it is.
#[derive(Debug, Clone, PartialEq)]
pub enum Glyph {
    /// One of [`Icon`].
    Icon(Icon),
    /// A single-color SVG, tinted when drawn.
    Svg(svg::Handle),
    /// A PNG or JPEG, never tinted.
    Image(image::Handle),
}

impl From<Icon> for Glyph {
    fn from(icon: Icon) -> Self {
        Self::Icon(icon)
    }
}

/// The glyph saved under `name`: a [`WORKSPACE_ICONS`] name, a [`library`] icon's, or
/// `file:<name>` for a file in the custom icons folder ([`set_custom_dir`]). `None` for no name
/// and for names that are none of these (a deleted file stays `Some`: it draws as nothing).
pub fn named_glyph(name: Option<&str>) -> Option<Glyph> {
    let name = name?;
    if let Some((_, icon)) = WORKSPACE_ICONS.iter().find(|(n, _)| *n == name) {
        return Some(Glyph::Icon(*icon));
    }
    if let Some(file) = name.strip_prefix(CUSTOM_PREFIX) {
        let path = custom_dir()?.join(file);
        return Some(if is_svg(&path) {
            Glyph::Svg(svg::Handle::from_path(path))
        } else {
            Glyph::Image(image::Handle::from_path(path))
        });
    }
    library_handle(name).map(Glyph::Svg)
}

/// `glyph` as a `size`-pixel square at `opacity`, an SVG tinted `color`. Pictures keep their
/// own colors, fitted inside the square.
pub fn glyph<'a, M: 'a>(glyph: Glyph, size: f32, color: Color, opacity: f32) -> Element<'a, M> {
    match glyph {
        Glyph::Icon(i) => icon(i, size, color).opacity(opacity).into(),
        Glyph::Svg(handle) => svg(handle)
            .width(Length::Fixed(size))
            .height(Length::Fixed(size))
            .opacity(opacity)
            .style(move |_, _| svg::Style { color: Some(color) })
            .into(),
        Glyph::Image(handle) => image(handle)
            .width(Length::Fixed(size))
            .height(Length::Fixed(size))
            .opacity(opacity)
            .into(),
    }
}

/// What a saved icon name starts with when it's a file in the custom icons folder.
pub const CUSTOM_PREFIX: &str = "file:";

/// The folder of icons the user added (`icons/` next to `config.toml`), set once at startup.
static CUSTOM_DIR: OnceLock<PathBuf> = OnceLock::new();

/// Sets where `file:` icons are looked up and saved. Only the first call counts.
pub fn set_custom_dir(dir: PathBuf) {
    let _ = CUSTOM_DIR.set(dir);
}

/// The custom icons folder, once [`set_custom_dir`] has run.
pub fn custom_dir() -> Option<&'static Path> {
    CUSTOM_DIR.get().map(PathBuf::as_path)
}

/// Whether `path` is an SVG, by its extension.
pub fn is_svg(path: &Path) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("svg"))
}

/// The icons the user added, as the names to save (`file:<name>`), newest first.
pub fn custom_icons() -> Vec<String> {
    let Some(dir) = custom_dir() else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<(std::time::SystemTime, String)> = entries
        .filter_map(Result::ok)
        .filter(|e| {
            let p = e.path();
            p.is_file()
                && p.extension().is_some_and(|x| {
                    ["svg", "png", "jpg", "jpeg"]
                        .iter()
                        .any(|ok| x.eq_ignore_ascii_case(ok))
                })
        })
        .filter_map(|e| {
            let modified = e.metadata().and_then(|m| m.modified()).ok()?;
            Some((modified, e.file_name().to_string_lossy().into_owned()))
        })
        .collect();
    files.sort_by_key(|f| std::cmp::Reverse(f.0));
    files
        .into_iter()
        .map(|(_, f)| format!("{CUSTOM_PREFIX}{f}"))
        .collect()
}

/// One icon of the bundled library.
#[derive(Debug)]
pub struct LibraryIcon {
    /// Its Lucide name, which is also the name saved with an item, e.g. `bot`.
    pub name: &'static str,
    /// Words it's also found by, comma separated.
    pub tags: &'static str,
    /// The SVG's elements, inside its `<svg>`.
    body: &'static str,
}

impl LibraryIcon {
    /// Whether `query` (lowercased) is in its name or tags.
    pub fn matches(&self, query: &str) -> bool {
        self.name.contains(query) || self.tags.contains(query)
    }
}

/// `library.tsv`: every Lucide icon (2,000 and more) as name, tags and SVG body, one per line.
/// See `references/rendering-and-text.md` for how it's made.
const LIBRARY: &str = include_str!("../assets/icons/library.tsv");

/// The bundled icons, parsed once, in name order.
pub fn library() -> &'static [LibraryIcon] {
    static PARSED: LazyLock<Vec<LibraryIcon>> = LazyLock::new(|| {
        LIBRARY
            .lines()
            .filter(|l| !l.starts_with('#') && !l.is_empty())
            .filter_map(|line| {
                let mut parts = line.splitn(3, '\t');
                Some(LibraryIcon {
                    name: parts.next()?,
                    tags: parts.next()?,
                    body: parts.next()?,
                })
            })
            .collect()
    });
    &PARSED
}

/// The SVG handle of the library icon `name`, made once per icon (so iced's svg cache, which
/// keys on the handle, rasterizes it once) and drawn at the UI's 1.5 stroke.
fn library_handle(name: &str) -> Option<svg::Handle> {
    static HANDLES: LazyLock<Mutex<HashMap<&'static str, svg::Handle>>> =
        LazyLock::new(|| Mutex::new(HashMap::new()));
    let icon = library().iter().find(|i| i.name == name)?;
    let mut handles = HANDLES.lock().unwrap_or_else(|e| e.into_inner());
    let handle = handles.entry(icon.name).or_insert_with(|| {
        let source = format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"24\" height=\"24\" \
             viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.5\" \
             stroke-linecap=\"round\" stroke-linejoin=\"round\">{}</svg>",
            icon.body
        );
        svg::Handle::from_memory(source.into_bytes())
    });
    Some(handle.clone())
}

/// `icon` as a `size`-pixel square SVG, tinted `color` (the color's alpha is ignored; fade with
/// `.opacity(..)`).
pub fn icon<'a>(icon: Icon, size: f32, color: Color) -> Svg<'a> {
    svg(handle(icon))
        .width(Length::Fixed(size))
        .height(Length::Fixed(size))
        .style(move |_, _| svg::Style { color: Some(color) })
}
