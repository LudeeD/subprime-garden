use std::sync::OnceLock;

use pulldown_cmark::{CodeBlockKind, CowStr, Event, Options, Parser, Tag, TagEnd};
use syntect::highlighting::{Theme, ThemeSet};
use syntect::parsing::SyntaxSet;

use crate::config::MarkdownConfig;

static SYNTAX_SET: OnceLock<SyntaxSet> = OnceLock::new();
static THEME: OnceLock<Theme> = OnceLock::new();

fn syntax_set() -> &'static SyntaxSet {
    SYNTAX_SET.get_or_init(SyntaxSet::load_defaults_newlines)
}

/// Fixed dark theme for code blocks regardless of the site's own light/dark
/// mode — syntax highlighting is baked into the HTML at save time (see
/// `render` below), so it can't react to `prefers-color-scheme` at request
/// time. To change it, edit this key and run `subprime-garden rerender`
/// (see THEMING.md).
fn theme() -> &'static Theme {
    THEME.get_or_init(|| {
        let ts = ThemeSet::load_defaults();
        ts.themes["base16-ocean.dark"].clone()
    })
}

pub struct Rendered {
    pub html: String,
    pub excerpt: String,
    pub content_hash: String,
}

/// Renders markdown to HTML once, at write time — requests never invoke this.
pub fn render(markdown: &str, config: &MarkdownConfig) -> Rendered {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);
    if config.footnotes {
        options.insert(Options::ENABLE_FOOTNOTES);
    }
    if config.smart_punctuation {
        options.insert(Options::ENABLE_SMART_PUNCTUATION);
    }

    let events: Vec<Event> = Parser::new_ext(markdown, options).collect();
    let events = assign_heading_ids(events);
    let events = rewrite_image_urls(events);
    let events = if config.syntax_highlighting {
        highlight_code_blocks(events)
    } else {
        events
    };

    let mut html = String::new();
    pulldown_cmark::html::push_html(&mut html, events.into_iter());
    let html = add_lazy_image_loading(&html);

    let excerpt = derive_excerpt(markdown);
    let content_hash = blake3::hash(html.as_bytes()).to_hex().to_string();

    Rendered {
        html,
        excerpt,
        content_hash,
    }
}

/// Gives every heading a stable, unique, unicode-aware id for anchor links.
fn assign_heading_ids(events: Vec<Event>) -> Vec<Event> {
    let mut used = std::collections::HashSet::new();
    let mut out = Vec::with_capacity(events.len());
    let mut i = 0;
    while i < events.len() {
        if let Event::Start(Tag::Heading {
            level,
            classes,
            attrs,
            ..
        }) = &events[i]
        {
            let mut text = String::new();
            let mut j = i + 1;
            let mut depth = 0i32;
            while j < events.len() {
                match &events[j] {
                    Event::Text(t) | Event::Code(t) => text.push_str(t),
                    Event::End(TagEnd::Heading(_)) if depth == 0 => break,
                    Event::Start(_) => depth += 1,
                    Event::End(_) => depth -= 1,
                    _ => {}
                }
                j += 1;
            }

            let base = {
                let s = slug::slugify(&text);
                if s.is_empty() {
                    "section".to_string()
                } else {
                    s
                }
            };
            let mut candidate = base.clone();
            let mut n = 2;
            while used.contains(&candidate) {
                candidate = format!("{base}-{n}");
                n += 1;
            }
            used.insert(candidate.clone());

            out.push(Event::Start(Tag::Heading {
                level: *level,
                id: Some(CowStr::from(candidate)),
                classes: classes.clone(),
                attrs: attrs.clone(),
            }));
            i += 1;
        } else {
            out.push(events[i].clone());
            i += 1;
        }
    }
    out
}

/// Bare relative image paths (`foo.png`) become `/media/foo.png`; anything
/// absolute, schemed, or a data URI passes through untouched.
fn rewrite_image_urls(events: Vec<Event>) -> Vec<Event> {
    events
        .into_iter()
        .map(|event| match event {
            Event::Start(Tag::Image {
                link_type,
                dest_url,
                title,
                id,
            }) => Event::Start(Tag::Image {
                link_type,
                dest_url: CowStr::from(rewrite_image_url(&dest_url)),
                title,
                id,
            }),
            other => other,
        })
        .collect()
}

fn rewrite_image_url(url: &str) -> String {
    if url.starts_with("http://")
        || url.starts_with("https://")
        || url.starts_with('/')
        || url.starts_with("data:")
    {
        url.to_string()
    } else {
        format!("/media/{url}")
    }
}

fn highlight_code_blocks(events: Vec<Event>) -> Vec<Event> {
    let ss = syntax_set();
    let theme = theme();
    let mut out = Vec::with_capacity(events.len());
    let mut i = 0;
    while i < events.len() {
        if let Event::Start(Tag::CodeBlock(kind)) = &events[i] {
            let lang = match kind {
                CodeBlockKind::Fenced(lang) => lang.to_string(),
                CodeBlockKind::Indented => String::new(),
            };
            let mut code = String::new();
            let mut j = i + 1;
            while j < events.len() {
                match &events[j] {
                    Event::Text(t) => code.push_str(t),
                    Event::End(TagEnd::CodeBlock) => break,
                    _ => {}
                }
                j += 1;
            }

            let syntax = ss
                .find_syntax_by_token(&lang)
                .unwrap_or_else(|| ss.find_syntax_plain_text());
            let highlighted =
                syntect::html::highlighted_html_for_string(&code, ss, syntax, theme)
                    .unwrap_or_else(|_| format!("<pre><code>{}</code></pre>", escape_html(&code)));

            out.push(Event::Html(CowStr::from(highlighted)));
            i = j + 1; // skip past End(CodeBlock)
        } else {
            out.push(events[i].clone());
            i += 1;
        }
    }
    out
}

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// `<img>` has no attrs slot in pulldown-cmark's event model, so this is a
/// targeted post-process rather than an event-level rewrite. Safe because the
/// HTML is our own trusted output, not third-party markup.
fn add_lazy_image_loading(html: &str) -> String {
    html.replace("<img ", "<img loading=\"lazy\" decoding=\"async\" ")
}

fn derive_excerpt(markdown: &str) -> String {
    let parser = Parser::new_ext(
        markdown,
        Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TABLES,
    );
    let mut text = String::new();
    let mut in_first_paragraph = false;
    let mut done = false;

    for event in parser {
        if done {
            break;
        }
        match event {
            Event::Start(Tag::Paragraph) => in_first_paragraph = true,
            Event::End(TagEnd::Paragraph) => {
                if in_first_paragraph {
                    done = true;
                }
            }
            Event::Text(t) | Event::Code(t) if in_first_paragraph => text.push_str(&t),
            Event::SoftBreak | Event::HardBreak if in_first_paragraph => text.push(' '),
            _ => {}
        }
    }

    truncate_at_word_boundary(text.trim(), 200)
}

fn truncate_at_word_boundary(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let truncated: String = s.chars().take(max_chars).collect();
    match truncated.rsplit_once(' ') {
        Some((head, _)) => format!("{head}\u{2026}"),
        None => format!("{truncated}\u{2026}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> MarkdownConfig {
        MarkdownConfig {
            syntax_highlighting: true,
            smart_punctuation: true,
            footnotes: true,
        }
    }

    #[test]
    fn headings_get_stable_unique_ids() {
        let r = render("# Hello World\n\nBody.\n\n## Hello World\n", &cfg());
        assert!(r.html.contains(r#"id="hello-world""#));
        assert!(r.html.contains(r#"id="hello-world-2""#));
    }

    #[test]
    fn bare_image_paths_are_rewritten_to_media() {
        let r = render("![alt](photo.png)", &cfg());
        assert!(r.html.contains(r#"src="/media/photo.png""#));
    }

    #[test]
    fn absolute_and_remote_image_paths_pass_through() {
        let r = render("![a](/already/there.png) ![b](https://example.com/x.png)", &cfg());
        assert!(r.html.contains(r#"src="/already/there.png""#));
        assert!(r.html.contains(r#"src="https://example.com/x.png""#));
    }

    #[test]
    fn images_get_lazy_loading_attrs() {
        let r = render("![alt](photo.png)", &cfg());
        assert!(r.html.contains(r#"<img loading="lazy" decoding="async""#));
    }

    #[test]
    fn code_blocks_are_highlighted_at_save_time() {
        let r = render("```rust\nfn main() {}\n```", &cfg());
        assert!(r.html.contains("<pre"));
        assert!(r.html.contains("span"));
    }

    #[test]
    fn excerpt_is_first_paragraph_plain_text() {
        let r = render("# Title\n\nThis is the **first** paragraph.\n\nSecond paragraph.", &cfg());
        assert_eq!(r.excerpt, "This is the first paragraph.");
    }

    #[test]
    fn content_hash_is_stable_for_identical_html() {
        let a = render("# Same\n\nBody.", &cfg());
        let b = render("# Same\n\nBody.", &cfg());
        assert_eq!(a.content_hash, b.content_hash);
    }
}
