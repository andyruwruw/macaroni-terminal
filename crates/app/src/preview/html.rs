//! Markdown, diagram and media files to the HTML a file tab's page shows.
//!
//! Markdown goes through `pulldown-cmark` with GitHub's extensions (tables, task lists,
//! footnotes, strikethrough, alerts) and math. Code blocks are highlighted here with
//! `syntect` (the same grammars and themes as the text view); blocks in a diagram language
//! become a figure holding their source, which the page's script draws when that viewer
//! extension is installed, and which otherwise stays code with a note saying what to
//! download. Math becomes `mx-math` spans the script typesets with KaTeX. Raw HTML passes
//! through; the page's content security policy keeps it from running scripts.
//!
//! Blocking and slow on big files (highlighting), so it runs off the UI thread with the read.

use std::sync::LazyLock;

use pulldown_cmark::{CodeBlockKind, CowStr, Event, Options, Parser, Tag, TagEnd};
use syntect::parsing::SyntaxSet;
use two_face::theme::{EmbeddedLazyThemeSet, EmbeddedThemeName};

use crate::viewers::Extension;

/// The grammars, loaded once.
static SYNTAXES: LazyLock<SyntaxSet> = LazyLock::new(two_face::syntax::extra_newlines);

/// The themes, loaded once.
static THEMES: LazyLock<EmbeddedLazyThemeSet> = LazyLock::new(two_face::theme::extra);

/// Code blocks longer than this aren't highlighted, which would hold the page up.
const HIGHLIGHT_MAX: usize = 256 * 1024;

/// What the HTML depends on besides the text.
#[derive(Debug, Clone)]
pub struct Render {
    /// Highlight for a dark background.
    pub dark: bool,
    /// The viewer extensions that are installed.
    pub installed: Vec<Extension>,
}

/// Markdown `text` as HTML for the page's body.
pub fn markdown(text: &str, options: &Render) -> String {
    let parser = Parser::new_ext(
        text,
        Options::ENABLE_TABLES
            | Options::ENABLE_FOOTNOTES
            | Options::ENABLE_STRIKETHROUGH
            | Options::ENABLE_TASKLISTS
            | Options::ENABLE_HEADING_ATTRIBUTES
            | Options::ENABLE_MATH
            | Options::ENABLE_GFM,
    );
    let mut events = Vec::new();
    // The code block being read: its language and text so far.
    let mut block: Option<(String, String)> = None;
    for event in parser {
        match event {
            Event::Start(Tag::CodeBlock(kind)) => {
                let language = match kind {
                    CodeBlockKind::Fenced(info) => {
                        info.split_whitespace().next().unwrap_or("").to_owned()
                    }
                    CodeBlockKind::Indented => String::new(),
                };
                block = Some((language, String::new()));
            }
            Event::Text(text) if block.is_some() => {
                if let Some((_, code)) = &mut block {
                    code.push_str(&text);
                }
            }
            Event::End(TagEnd::CodeBlock) => {
                if let Some((language, code)) = block.take() {
                    events.push(Event::Html(CowStr::from(code_block(
                        &language, &code, options,
                    ))));
                }
            }
            Event::InlineMath(tex) => events.push(Event::InlineHtml(CowStr::from(format!(
                "<span class=\"mx-math\">{}</span>",
                escape(&tex)
            )))),
            Event::DisplayMath(tex) => events.push(Event::InlineHtml(CowStr::from(format!(
                "<span class=\"mx-math mx-display\">{}</span>",
                escape(&tex)
            )))),
            event => events.push(event),
        }
    }
    let mut html = String::with_capacity(text.len() * 3 / 2);
    pulldown_cmark::html::push_html(&mut html, events.into_iter());
    html
}

/// A whole diagram file (`extension`'s language) as the page's body.
pub fn diagram(extension: Extension, text: &str, options: &Render) -> String {
    diagram_block(extension, text, options)
}

/// A media file as the page's body: the picture, or a player for a video or sound. `name` is
/// the file's name (the page's base is its folder) and `tag` changes with the file, so a new
/// version loads afresh. Something the web view can't decode says so (the page's script).
pub fn media(media: crate::file_view::Media, name: &str, tag: &str) -> String {
    use crate::file_view::Media;
    let src = escape(&format!("{}?v={tag}", super::encode(name)));
    let name = escape(name);
    match media {
        Media::Picture => {
            format!("<figure class=\"mx-media\"><img src=\"{src}\" alt=\"{name}\"></figure>")
        }
        Media::Video => format!(
            "<figure class=\"mx-media\"><video src=\"{src}\" controls preload=\"metadata\"></video></figure>"
        ),
        Media::Audio => format!(
            "<figure class=\"mx-media mx-audio\"><figcaption>{name}</figcaption><audio src=\"{src}\" \
             controls preload=\"metadata\"></audio></figure>"
        ),
    }
}

/// A fenced code block: a diagram, display math, or highlighted code.
fn code_block(language: &str, code: &str, options: &Render) -> String {
    if language.eq_ignore_ascii_case("math") {
        return format!(
            "<p><span class=\"mx-math mx-display\">{}</span></p>\n",
            escape(code)
        );
    }
    if let Some(extension) = Extension::for_language(language) {
        return diagram_block(extension, code, options);
    }
    format!(
        "<pre class=\"mx-code\"><code>{}</code></pre>\n",
        highlight(language, code, options.dark)
    )
}

/// A diagram's figure: its source, which the page's script replaces with the drawing when
/// `extension` is installed; otherwise the source stays, with a note.
fn diagram_block(extension: Extension, code: &str, options: &Render) -> String {
    let source = format!(
        "<pre class=\"mx-code mx-source\"><code>{}</code></pre>",
        escape(code)
    );
    if options.installed.contains(&extension) {
        format!(
            "<figure class=\"mx-diagram\" data-kind=\"{}\">{source}</figure>\n",
            extension.id()
        )
    } else {
        format!(
            "<figure class=\"mx-diagram mx-missing\">{source}<figcaption>Download {} in \
             Settings &gt; Files to draw this.</figcaption></figure>\n",
            extension.name()
        )
    }
}

/// `code` as HTML spans colored for `language` (an extension or name, any case); plain
/// escaped text when the language is unknown or the code is long.
fn highlight(language: &str, code: &str, dark: bool) -> String {
    let syntax = (!language.is_empty() && code.len() <= HIGHLIGHT_MAX)
        .then(|| {
            SYNTAXES
                .find_syntax_by_token(language)
                .or_else(|| SYNTAXES.find_syntax_by_token(&language.to_ascii_lowercase()))
        })
        .flatten();
    let Some(syntax) = syntax else {
        return escape(code);
    };
    let theme = THEMES.get(if dark {
        EmbeddedThemeName::Base16OceanDark
    } else {
        EmbeddedThemeName::InspiredGithub
    });
    let mut highlighter = syntect::easy::HighlightLines::new(syntax, theme);
    let mut html = String::with_capacity(code.len() * 2);
    for line in syntect::util::LinesWithEndings::from(code) {
        let Ok(regions) = highlighter.highlight_line(line, &SYNTAXES) else {
            return escape(code);
        };
        // The block's own background is the page's, so the theme's is left out.
        match syntect::html::styled_line_to_highlighted_html(
            &regions,
            syntect::html::IncludeBackground::No,
        ) {
            Ok(spans) => html.push_str(&spans),
            Err(_) => return escape(code),
        }
    }
    html
}

/// `text` safe inside HTML text and attribute values.
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Options with `installed` and a light background.
    fn with(installed: &[Extension]) -> Render {
        Render {
            dark: false,
            installed: installed.to_vec(),
        }
    }

    #[test]
    fn diagrams_wait_for_their_viewer() {
        let text = "```mermaid\ngraph TD; A-->B\n```\n";
        let drawn = markdown(text, &with(&[Extension::Mermaid]));
        assert!(drawn.contains("data-kind=\"mermaid\""));
        assert!(drawn.contains("A--&gt;B"));
        let missing = markdown(text, &with(&[]));
        assert!(!missing.contains("data-kind"));
        assert!(missing.contains("Download Mermaid"));
    }

    #[test]
    fn math_becomes_spans_and_code_is_highlighted() {
        let html = markdown(
            "Euler: $e^{i\\pi} = -1$\n\n$$x<y$$\n\n```rust\nfn main() {}\n```\n",
            &with(&[]),
        );
        assert!(html.contains("<span class=\"mx-math\">e^{i\\pi} = -1</span>"));
        assert!(html.contains("<span class=\"mx-math mx-display\">x&lt;y</span>"));
        assert!(html.contains("<pre class=\"mx-code\"><code><span style="));
        assert!(html.contains("main"));
    }

    #[test]
    fn unknown_languages_and_raw_text_are_escaped() {
        let html = markdown("```nosuchlanguage\n<b>&\n```\n", &with(&[]));
        assert!(html.contains("&lt;b&gt;&amp;"));
        let html = diagram(Extension::D2, "a -> b", &with(&[Extension::D2]));
        assert!(html.contains("data-kind=\"d2\""));
        assert!(html.contains("a -&gt; b"));
    }

    #[test]
    fn media_gets_a_player() {
        use crate::file_view::Media;
        let video = media(Media::Video, "my clip.mp4", "ab");
        assert!(video.contains("<video src=\"my%20clip.mp4?v=ab\" controls"));
        let audio = media(Media::Audio, "a<b.mp3", "1");
        assert!(audio.contains("<figcaption>a&lt;b.mp3</figcaption>"));
        assert!(audio.contains("<audio src=\"a%3Cb.mp3?v=1\""));
        assert!(media(Media::Picture, "x.gif", "1").contains("<img src=\"x.gif?v=1\""));
    }

    #[test]
    fn github_extras_render() {
        let html = markdown(
            "| a | b |\n|---|---|\n| 1 | 2 |\n\n- [x] done\n\n~~gone~~\n",
            &with(&[]),
        );
        assert!(html.contains("<table>"));
        assert!(html.contains("checkbox"));
        assert!(html.contains("<del>gone</del>"));
    }
}
