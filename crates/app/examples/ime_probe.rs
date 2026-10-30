//! Manual probe: a custom iced widget that behaves like a terminal input line.
//!
//! - Font fallback: sample lines with CJK, emoji and Nerd Font symbols drawn in the monospace font.
//! - IME: the widget requests `Purpose::Terminal`, reports its caret rect so the candidate window
//!   can be placed there, draws preedit inline and underlined, and only "sends" committed text.
//!   Every IME and key event is logged below the widget and to stdout.
//!
//! It's standalone: it doesn't load the app's bundled fonts and draws with the system
//! `Font::MONOSPACE`, so what it shows is iced's own fallback, not what `TerminalView` draws.
//!
//! Usage: cargo run -p macaroni --example ime_probe

use iced::advanced::input_method::{self, InputMethod, Preedit, Purpose};
use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer::{self, Quad, Renderer as _};
use iced::advanced::text::{self, Paragraph as _, Renderer as _, Text};
use iced::advanced::widget::{self, Tree, Widget};
use iced::advanced::{Clipboard, Shell, mouse};
use iced::keyboard::{self, key};
use iced::widget::{column, container, scrollable, text as label};
use iced::{
    Border, Color, Element, Event, Fill, Font, Length, Pixels, Point, Rectangle, Size, Theme,
    window,
};

/// Text size, logical pixels.
const FONT_SIZE: f32 = 18.0;
/// Height of each line, logical pixels.
const LINE_HEIGHT: f32 = 26.0;
/// Space around the widget's contents and the log.
const PADDING: f32 = 12.0;
/// Drawn before the input line.
const PROMPT: &str = "> ";

/// Lines that exercise font fallback, drawn above the input line.
const SAMPLES: &[&str] = &[
    "ASCII    The quick brown fox jumps over the lazy dog 0123456789",
    "Box      ┌─┬─┐ │ │ │ └─┴─┘ ░▒▓█",
    "CJK      中文 日本語のテキスト 한국어",
    "Emoji    🎉 👍🏽 🧑‍💻 ❤️",
    "Nerd     \u{e0b0} \u{e0a0} \u{f418} \u{f07b}",
];

/// Opens the probe window.
fn main() -> iced::Result {
    iced::application(Probe::default, Probe::update, Probe::view)
        .title("macaroni: IME + font fallback probe")
        .window_size(Size::new(900.0, 600.0))
        .theme(Theme::Dark)
        .run()
}

/// The probe app: just the event log.
#[derive(Default)]
struct Probe {
    /// Every logged line, oldest first (the view shows the newest 200, newest first).
    log: Vec<String>,
}

/// The probe's only message.
#[derive(Debug, Clone)]
enum Message {
    /// A line for the log.
    Log(String),
}

impl Probe {
    /// Appends the line to the log and prints it.
    fn update(&mut self, message: Message) {
        let Message::Log(line) = message;
        println!("{line}");
        self.log.push(line);
    }

    /// The input widget over the scrolling log.
    fn view(&self) -> Element<'_, Message> {
        let log = self
            .log
            .iter()
            .rev()
            .take(200)
            .fold(column![].spacing(2), |col, line| {
                col.push(label(line.as_str()).font(Font::MONOSPACE).size(13))
            });
        column![
            TerminalLine::new(Message::Log),
            container(scrollable(log).width(Fill).height(Fill)).padding(PADDING),
        ]
        .into()
    }
}

/// The widget under test: the sample lines, then a prompt and an input line.
struct TerminalLine<M> {
    /// Builds the message for a log line.
    on_log: fn(String) -> M,
}

impl<M> TerminalLine<M> {
    /// A widget that logs through `on_log`.
    fn new(on_log: fn(String) -> M) -> Self {
        Self { on_log }
    }
}

/// The widget's state in the tree.
#[derive(Default)]
struct State {
    /// What has been "sent": committed IME text and typed characters.
    committed: String,
    /// The IME's composition, drawn inline and never sent.
    preedit: Option<Preedit>,
    /// Whether the window has focus; the IME is only requested while it does.
    window_focused: bool,
    /// Whether the first IME request was logged yet.
    ime_requested: bool,
}

/// How wide `content` is when shaped like the input line.
fn text_width(content: &str) -> f32 {
    <iced::Renderer as text::Renderer>::Paragraph::with_text(Text {
        content,
        bounds: Size::INFINITE,
        size: Pixels(FONT_SIZE),
        line_height: text::LineHeight::Absolute(Pixels(LINE_HEIGHT)),
        font: Font::MONOSPACE,
        align_x: text::Alignment::Left,
        align_y: iced::alignment::Vertical::Top,
        shaping: text::Shaping::Advanced,
        wrapping: text::Wrapping::None,
    })
    .min_width()
}

/// One line of owned text (one-off draws need owned text; see `render.rs`'s `owned_text`).
fn line(content: String, bounds: Size) -> Text<String> {
    Text {
        content,
        bounds,
        size: Pixels(FONT_SIZE),
        line_height: text::LineHeight::Absolute(Pixels(LINE_HEIGHT)),
        font: Font::MONOSPACE,
        align_x: text::Alignment::Left,
        align_y: iced::alignment::Vertical::Top,
        shaping: text::Shaping::Advanced,
        wrapping: text::Wrapping::None,
    }
}

impl<M> TerminalLine<M> {
    /// The input line's top: one blank line below the samples.
    fn input_row_y(bounds: Rectangle) -> f32 {
        bounds.y + PADDING + LINE_HEIGHT * (SAMPLES.len() as f32 + 1.0)
    }

    /// The caret at the end of the committed text, as reported to the IME.
    fn caret(state: &State, bounds: Rectangle) -> Rectangle {
        let x = bounds.x + PADDING + text_width(&format!("{PROMPT}{}", state.committed));
        Rectangle::new(
            Point::new(x, Self::input_row_y(bounds)),
            Size::new(1.0, LINE_HEIGHT),
        )
    }
}

impl<M: Clone> Widget<M, Theme, iced::Renderer> for TerminalLine<M> {
    fn tag(&self) -> widget::tree::Tag {
        widget::tree::Tag::of::<State>()
    }

    fn state(&self) -> widget::tree::State {
        widget::tree::State::new(State {
            window_focused: true,
            ..State::default()
        })
    }

    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Shrink)
    }

    fn layout(
        &mut self,
        _tree: &mut Tree,
        _renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let height = PADDING * 2.0 + LINE_HEIGHT * (SAMPLES.len() as f32 + 2.0);
        layout::Node::new(limits.resolve(Length::Fill, Length::Fixed(height), Size::ZERO))
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        _renderer: &iced::Renderer,
        _clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, M>,
        _viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_mut::<State>();
        let log = |shell: &mut Shell<'_, M>, line: String| shell.publish((self.on_log)(line));

        match event {
            Event::InputMethod(ime) => {
                match ime {
                    input_method::Event::Opened => state.preedit = Some(Preedit::new()),
                    input_method::Event::Closed => state.preedit = None,
                    input_method::Event::Preedit(content, selection) => {
                        state.preedit = Some(Preedit {
                            content: content.clone(),
                            selection: selection.clone(),
                            text_size: Some(Pixels(FONT_SIZE)),
                        });
                    }
                    input_method::Event::Commit(committed) => {
                        state.committed.push_str(committed);
                        log(shell, format!("PTY <- {committed:?} (commit)"));
                    }
                }
                log(shell, format!("ime  {ime:?}"));
                shell.capture_event();
                shell.request_redraw();
            }
            Event::Keyboard(keyboard::Event::KeyPressed {
                key,
                text,
                modifiers,
                ..
            }) => {
                log(
                    shell,
                    format!("key  {key:?} text={text:?} mods={modifiers:?}"),
                );
                if state
                    .preedit
                    .as_ref()
                    .is_some_and(|p| !p.content.is_empty())
                {
                    return;
                }
                match key {
                    keyboard::Key::Named(key::Named::Backspace) => {
                        state.committed.pop();
                    }
                    keyboard::Key::Named(key::Named::Enter) => {
                        log(shell, format!("PTY <- line {:?}", state.committed));
                        state.committed.clear();
                    }
                    _ => {
                        if let Some(text) =
                            text.as_ref().filter(|t| !t.chars().any(char::is_control))
                        {
                            state.committed.push_str(text);
                            log(shell, format!("PTY <- {text:?} (key)"));
                        }
                    }
                }
                shell.capture_event();
                shell.request_redraw();
            }
            Event::Window(window::Event::Focused) => {
                state.window_focused = true;
                shell.request_redraw();
            }
            Event::Window(window::Event::Unfocused) => state.window_focused = false,
            // The IME request lasts one frame, so it's renewed on every redraw.
            Event::Window(window::Event::RedrawRequested(_)) if state.window_focused => {
                let caret = Self::caret(state, layout.bounds());
                if !state.ime_requested {
                    state.ime_requested = true;
                    log(shell, format!("ime  requested, caret at {caret:?}"));
                }
                shell.request_input_method(&InputMethod::Enabled {
                    cursor: caret,
                    purpose: Purpose::Terminal,
                    preedit: state.preedit.as_ref().map(Preedit::as_ref),
                });
            }
            _ => {}
        }
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut iced::Renderer,
        _theme: &Theme,
        _style: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        _viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_ref::<State>();
        let bounds = layout.bounds();
        let fg = Color::from_rgb8(0xd8, 0xde, 0xe9);
        let row = Size::new(bounds.width - PADDING * 2.0, LINE_HEIGHT);

        renderer.fill_quad(
            Quad {
                bounds,
                ..Quad::default()
            },
            Color::from_rgb8(0x1e, 0x22, 0x2a),
        );

        for (i, sample) in SAMPLES.iter().enumerate() {
            let y = bounds.y + PADDING + LINE_HEIGHT * i as f32;
            renderer.fill_text(
                line((*sample).to_owned(), row),
                Point::new(bounds.x + PADDING, y),
                fg,
                bounds,
            );
        }

        let y = Self::input_row_y(bounds);
        let head = format!("{PROMPT}{}", state.committed);
        renderer.fill_text(
            line(head, row),
            Point::new(bounds.x + PADDING, y),
            fg,
            bounds,
        );

        let caret = Self::caret(state, bounds);
        let preedit = state.preedit.as_ref().map_or("", |p| p.content.as_str());
        let preedit_width = text_width(preedit);
        if !preedit.is_empty() {
            let accent = Color::from_rgb8(0xe5, 0xc0, 0x7b);
            renderer.fill_text(
                line(preedit.to_owned(), row),
                caret.position(),
                accent,
                bounds,
            );
            renderer.fill_quad(
                Quad {
                    bounds: Rectangle::new(
                        Point::new(caret.x, y + LINE_HEIGHT - 3.0),
                        Size::new(preedit_width, 1.5),
                    ),
                    ..Quad::default()
                },
                accent,
            );
        }

        let cell = text_width("M");
        renderer.fill_quad(
            Quad {
                bounds: Rectangle::new(
                    Point::new(caret.x + preedit_width, y + 2.0),
                    Size::new(cell, LINE_HEIGHT - 4.0),
                ),
                border: Border::default(),
                ..Quad::default()
            },
            Color {
                a: if state.window_focused { 0.8 } else { 0.3 },
                ..fg
            },
        );
    }
}

impl<'a, M: Clone + 'a> From<TerminalLine<M>> for Element<'a, M> {
    fn from(widget: TerminalLine<M>) -> Self {
        Element::new(widget)
    }
}
