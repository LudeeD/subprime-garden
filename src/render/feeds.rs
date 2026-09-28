use crate::db::models::Post;

use super::SiteView;

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// RSS 2.0 requires RFC 822 dates; Atom requires RFC 3339 (which is exactly
/// how we already store `published_at`, so Atom just reuses it as-is).
fn rfc822(iso: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(iso)
        .map(|dt| dt.with_timezone(&chrono::Utc).format("%a, %d %b %Y %H:%M:%S GMT").to_string())
        .unwrap_or_default()
}

fn post_url(site: &SiteView, post: &Post) -> String {
    format!("{}/{}", site.base_url.trim_end_matches('/'), post.slug)
}

pub fn atom_feed(site: &SiteView, posts: &[Post]) -> String {
    let updated = posts
        .first()
        .and_then(|p| p.published_at.as_deref())
        .unwrap_or("");

    let mut entries = String::new();
    for post in posts {
        let url = post_url(site, post);
        let published = post.published_at.as_deref().unwrap_or("");
        entries.push_str(&format!(
            "  <entry>\n\
             \x20   <title>{title}</title>\n\
             \x20   <link href=\"{url}\"/>\n\
             \x20   <id>{url}</id>\n\
             \x20   <published>{published}</published>\n\
             \x20   <updated>{updated}</updated>\n\
             \x20   <summary>{excerpt}</summary>\n\
             \x20   <content type=\"html\">{content}</content>\n\
             \x20 </entry>\n",
            title = xml_escape(&post.title),
            url = xml_escape(&url),
            published = published,
            updated = post.updated_at,
            excerpt = xml_escape(&post.excerpt),
            content = xml_escape(&post.html),
        ));
    }

    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n\
         <feed xmlns=\"http://www.w3.org/2005/Atom\">\n\
         \x20 <title>{title}</title>\n\
         \x20 <link href=\"{base}/feed.xml\" rel=\"self\"/>\n\
         \x20 <link href=\"{base}/\"/>\n\
         \x20 <id>{base}/</id>\n\
         \x20 <updated>{updated}</updated>\n\
         \x20 <author><name>{author}</name></author>\n\
         {entries}\
         </feed>\n",
        title = xml_escape(&site.title),
        author = xml_escape(&site.author),
        base = site.base_url.trim_end_matches('/'),
        updated = updated,
        entries = entries,
    )
}

pub fn rss_feed(site: &SiteView, posts: &[Post]) -> String {
    let mut items = String::new();
    for post in posts {
        let url = post_url(site, post);
        let pub_date = post
            .published_at
            .as_deref()
            .map(rfc822)
            .unwrap_or_default();
        items.push_str(&format!(
            "  <item>\n\
             \x20   <title>{title}</title>\n\
             \x20   <link>{url}</link>\n\
             \x20   <guid>{url}</guid>\n\
             \x20   <pubDate>{pub_date}</pubDate>\n\
             \x20   <description>{excerpt}</description>\n\
             \x20   <content:encoded>{content}</content:encoded>\n\
             \x20 </item>\n",
            title = xml_escape(&post.title),
            url = xml_escape(&url),
            pub_date = pub_date,
            excerpt = xml_escape(&post.excerpt),
            content = xml_escape(&post.html),
        ));
    }

    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n\
         <rss version=\"2.0\" xmlns:content=\"http://purl.org/rss/1.0/modules/content/\">\n\
         <channel>\n\
         \x20 <title>{title}</title>\n\
         \x20 <link>{base}/</link>\n\
         \x20 <description>{description}</description>\n\
         {items}\
         </channel>\n\
         </rss>\n",
        title = xml_escape(&site.title),
        base = site.base_url.trim_end_matches('/'),
        description = xml_escape(&site.description),
        items = items,
    )
}

/// Sitemap includes every publicly reachable post/page URL plus the two
/// static index-style routes; drafts are filtered by the caller (only
/// published rows are ever passed in).
pub fn sitemap_xml(site: &SiteView, posts_and_pages: &[Post]) -> String {
    let base = site.base_url.trim_end_matches('/');
    let mut urls = format!(
        "  <url><loc>{base}/</loc></url>\n  <url><loc>{base}/archive</loc></url>\n"
    );
    for post in posts_and_pages {
        let lastmod = post.updated_at.split('T').next().unwrap_or("").to_string();
        urls.push_str(&format!(
            "  <url><loc>{base}/{slug}</loc><lastmod>{lastmod}</lastmod></url>\n",
            base = base,
            slug = xml_escape(&post.slug),
            lastmod = lastmod,
        ));
    }

    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n\
         <urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">\n\
         {urls}\
         </urlset>\n"
    )
}
