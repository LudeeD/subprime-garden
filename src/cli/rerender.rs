use rusqlite::Connection;

use crate::config::MarkdownConfig;
use crate::content::markdown;
use crate::db::{media, posts};

/// Rebuilds stored HTML/excerpt for every post — needed after
/// changing markdown options or the syntax theme, since rendering only ever
/// happens at write time otherwise.
pub fn run(conn: &Connection, markdown_cfg: &MarkdownConfig) -> anyhow::Result<()> {
    let all = posts::list_all(conn, None)?;
    for post in &all {
        let rendered = markdown::render(&post.markdown, markdown_cfg, &|filename| {
            media::variant_lookup(conn, filename)
        });
        posts::update_rendered(conn, post.id, &rendered.html, &rendered.excerpt)?;
        println!("rerendered: {}", post.slug);
    }
    println!("\n{} posts rerendered", all.len());
    println!("restart the server to see these changes");
    Ok(())
}
