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
/// (see README.md, Theming).
fn theme() -> &'static Theme {
    THEME.get_or_init(|| {
        let ts = ThemeSet::load_defaults();
        ts.themes["base16-ocean.dark"].clone()
    })
}

pub struct Rendered {
    pub html: String,
    pub excerpt: String,
}

/// Renders markdown to HTML once, at write time — requests never invoke this.
///
/// `variant_lookup` maps a `/media/`-relative filename to its downscaled
/// webp variant filename, if one exists (see `media_store`) — used to swap
/// plain `<img>` markup for a `<picture>` element. Pass `&|_| None` where no
/// media table is available (e.g. isolated tests).
pub fn render(
    markdown: &str,
    config: &MarkdownConfig,
    variant_lookup: &dyn Fn(&str) -> Option<String>,
) -> Rendered {
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
    let events = process_images(events, variant_lookup);
    let events = if config.syntax_highlighting {
        highlight_code_blocks(events)
    } else {
        events
    };

    let mut html = String::new();
    pulldown_cmark::html::push_html(&mut html, events.into_iter());
    let html = add_lazy_image_loading(&html);

    let excerpt = derive_excerpt(markdown);

    Rendered { html, excerpt }
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

/// Rewrites bare relative image paths (`foo.png`) to `/media/foo.png`
/// (anything absolute, schemed, or a data URI passes through untouched), and
/// swaps in a `<picture>` element for any image with a downscaled webp
/// variant on record.
fn process_images<'a>(
    events: Vec<Event<'a>>,
    variant_lookup: &dyn Fn(&str) -> Option<String>,
) -> Vec<Event<'a>> {
    let mut out = Vec::with_capacity(events.len());
    let mut i = 0;
    while i < events.len() {
        if let Event::Start(Tag::Image {
            link_type,
            dest_url,
            title,
            id,
        }) = &events[i]
        {
            let link_type = *link_type;
            let title = title.clone();
            let id = id.clone();
            let dest = rewrite_image_url(dest_url);

            let mut alt = String::new();
            let mut j = i + 1;
            while j < events.len() {
                match &events[j] {
                    Event::Text(t) | Event::Code(t) => alt.push_str(t),
                    Event::End(TagEnd::Image) => break,
                    _ => {}
                }
                j += 1;
            }

            let variant = media_filename_from_media_path(&dest).and_then(variant_lookup);
            if let Some(variant_filename) = variant {
                let title_attr = if title.is_empty() {
                    String::new()
                } else {
                    format!(" title=\"{}\"", escape_attr(&title))
                };
                let html = format!(
                    "<picture><source srcset=\"/media/{variant_filename}\" type=\"image/webp\">\
                     <img src=\"{}\" alt=\"{}\"{title_attr}></picture>",
                    escape_attr(&dest),
                    escape_attr(&alt),
                );
                out.push(Event::Html(CowStr::from(html)));
            } else {
                out.push(Event::Start(Tag::Image {
                    link_type,
                    dest_url: CowStr::from(dest),
                    title,
                    id,
                }));
                out.extend(events[(i + 1)..=j].iter().cloned());
            }
            i = j + 1;
        } else {
            out.push(events[i].clone());
            i += 1;
        }
    }
    out
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

fn media_filename_from_media_path(path: &str) -> Option<&str> {
    path.strip_prefix("/media/")
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

fn escape_attr(s: &str) -> String {
    escape_html(s).replace('"', "&quot;")
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

    fn no_variants(_: &str) -> Option<String> {
        None
    }

    #[test]
    fn headings_get_stable_unique_ids() {
        let r = render("# Hello World\n\nBody.\n\n## Hello World\n", &cfg(), &no_variants);
        assert!(r.html.contains(r#"id="hello-world""#));
        assert!(r.html.contains(r#"id="hello-world-2""#));
    }

    #[test]
    fn bare_image_paths_are_rewritten_to_media() {
        let r = render("![alt](photo.png)", &cfg(), &no_variants);
        assert!(r.html.contains(r#"src="/media/photo.png""#));
    }

    #[test]
    fn absolute_and_remote_image_paths_pass_through() {
        let r = render(
            "![a](/already/there.png) ![b](https://example.com/x.png)",
            &cfg(),
            &no_variants,
        );
        assert!(r.html.contains(r#"src="/already/there.png""#));
        assert!(r.html.contains(r#"src="https://example.com/x.png""#));
    }

    #[test]
    fn images_get_lazy_loading_attrs() {
        let r = render("![alt](photo.png)", &cfg(), &no_variants);
        assert!(r.html.contains(r#"<img loading="lazy" decoding="async""#));
    }

    #[test]
    fn images_with_a_variant_become_picture_elements() {
        let r = render("![a photo](photo.png)", &cfg(), &|filename| {
            (filename == "photo.png").then(|| "abc123-1600.webp".to_string())
        });
        assert!(r.html.contains("<picture>"));
        assert!(r.html.contains(r#"srcset="/media/abc123-1600.webp""#));
        assert!(r.html.contains(r#"type="image/webp""#));
        assert!(r.html.contains(r#"src="/media/photo.png""#));
        assert!(r.html.contains(r#"alt="a photo""#));
        assert_eq!(r.html.matches(r#"loading="lazy""#).count(), 1);
    }

    #[test]
    fn code_blocks_are_highlighted_at_save_time() {
        let r = render("```rust\nfn main() {}\n```", &cfg(), &no_variants);
        assert!(r.html.contains("<pre"));
        assert!(r.html.contains("span"));
    }

    #[test]
    fn excerpt_is_first_paragraph_plain_text() {
        let r = render(
            "# Title\n\nThis is the **first** paragraph.\n\nSecond paragraph.",
            &cfg(),
            &no_variants,
        );
        assert_eq!(r.excerpt, "This is the first paragraph.");
    }
}
