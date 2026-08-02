use serde::Deserialize;

#[derive(Debug, Default, Clone)]
pub struct Frontmatter {
    pub title: Option<String>,
    pub date: Option<String>,
    pub slug: Option<String>,
    pub tags: Vec<String>,
    pub draft: Option<bool>,
    pub description: Option<String>,
}

pub struct ParsedFile {
    pub frontmatter: Frontmatter,
    pub body: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RawFrontmatter {
    title: Option<String>,
    #[serde(alias = "published_at")]
    date: Option<String>,
    slug: Option<String>,
    tags: Vec<String>,
    taxonomies: Option<RawTaxonomies>,
    draft: Option<bool>,
    description: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RawTaxonomies {
    tags: Vec<String>,
}

impl From<RawFrontmatter> for Frontmatter {
    fn from(raw: RawFrontmatter) -> Self {
        let mut tags = raw.tags;
        if let Some(taxonomies) = raw.taxonomies {
            tags.extend(taxonomies.tags);
        }
        Frontmatter {
            title: raw.title,
            date: raw.date,
            slug: raw.slug,
            tags,
            draft: raw.draft,
            description: raw.description,
        }
    }
}

/// Accepts YAML (`---`) or TOML (`+++`) frontmatter, common to Zola, Hugo,
/// and Jekyll exports. Files with neither delimiter are treated as having no
/// frontmatter at all — the whole file is the body.
pub fn parse(content: &str) -> ParsedFile {
    if let Some(rest) = strip_bom(content).strip_prefix("---") {
        if let Some(rest) = rest.strip_prefix("\r\n").or_else(|| rest.strip_prefix('\n')) {
            if let Some((block, body)) = split_at_delimiter_line(rest, "---") {
                let raw: RawFrontmatter = serde_yaml::from_str(block).unwrap_or_default();
                return ParsedFile {
                    frontmatter: raw.into(),
                    body: body.to_string(),
                };
            }
        }
    }
    if let Some(rest) = strip_bom(content).strip_prefix("+++") {
        if let Some(rest) = rest.strip_prefix("\r\n").or_else(|| rest.strip_prefix('\n')) {
            if let Some((block, body)) = split_at_delimiter_line(rest, "+++") {
                let raw: RawFrontmatter = toml::from_str(block).unwrap_or_default();
                return ParsedFile {
                    frontmatter: raw.into(),
                    body: body.to_string(),
                };
            }
        }
    }
    ParsedFile {
        frontmatter: Frontmatter::default(),
        body: content.to_string(),
    }
}

fn strip_bom(s: &str) -> &str {
    s.strip_prefix('\u{feff}').unwrap_or(s)
}

/// Finds the first line that is exactly `delim` and splits the string into
/// (everything before it, everything after it).
fn split_at_delimiter_line<'a>(s: &'a str, delim: &str) -> Option<(&'a str, &'a str)> {
    let mut offset = 0;
    for line in s.split_inclusive('\n') {
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed == delim {
            let block = &s[..offset];
            let body = &s[offset + line.len()..];
            return Some((block, body));
        }
        offset += line.len();
    }
    None
}

/// Resolves a frontmatter date string (RFC 3339, `YYYY-MM-DD`, or
/// `YYYY-MM-DD HH:MM:SS`) to our stored ISO 8601 format, falling back to
/// `fallback` (the file's mtime) when there's no date or it doesn't parse.
pub fn resolve_date(date_str: Option<&str>, fallback: std::time::SystemTime) -> String {
    if let Some(s) = date_str {
        if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
            return format_utc(dt.with_timezone(&chrono::Utc));
        }
        if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S") {
            return format_utc(chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(
                dt,
                chrono::Utc,
            ));
        }
        if let Ok(d) = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d") {
            let dt = d.and_hms_opt(0, 0, 0).expect("midnight is always valid");
            return format_utc(chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(
                dt,
                chrono::Utc,
            ));
        }
    }
    format_utc(chrono::DateTime::<chrono::Utc>::from(fallback))
}

fn format_utc(dt: chrono::DateTime<chrono::Utc>) -> String {
    dt.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// Pulls a leading ATX H1 (`# Title`) out as the title, removing that line
/// from the body so it isn't rendered twice (once as `post.title`, once
/// inline). Only looks at the first non-blank line.
pub fn extract_h1_title(body: &str) -> (Option<String>, String) {
    let mut lines = body.lines();
    for (i, line) in lines.by_ref().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(title) = trimmed.strip_prefix("# ") {
            let rest: Vec<&str> = body.lines().skip(i + 1).collect();
            return (Some(title.trim().to_string()), rest.join("\n"));
        }
        break;
    }
    (None, body.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_yaml_frontmatter() {
        let input = "---\ntitle: Hello\ntags: [a, b]\ndraft: true\n---\nBody text.\n";
        let parsed = parse(input);
        assert_eq!(parsed.frontmatter.title.as_deref(), Some("Hello"));
        assert_eq!(parsed.frontmatter.tags, vec!["a", "b"]);
        assert_eq!(parsed.frontmatter.draft, Some(true));
        assert_eq!(parsed.body.trim(), "Body text.");
    }

    #[test]
    fn parses_toml_frontmatter_with_taxonomies() {
        let input = "+++\ntitle = \"Hi\"\n[taxonomies]\ntags = [\"x\", \"y\"]\n+++\nBody.\n";
        let parsed = parse(input);
        assert_eq!(parsed.frontmatter.title.as_deref(), Some("Hi"));
        assert_eq!(parsed.frontmatter.tags, vec!["x", "y"]);
    }

    #[test]
    fn accepts_published_at_alias_for_date() {
        let input = "---\npublished_at: 2024-01-15\n---\nBody.\n";
        let parsed = parse(input);
        assert_eq!(parsed.frontmatter.date.as_deref(), Some("2024-01-15"));
    }

    #[test]
    fn no_frontmatter_is_whole_file_as_body() {
        let input = "Just a plain markdown file.\n";
        let parsed = parse(input);
        assert!(parsed.frontmatter.title.is_none());
        assert_eq!(parsed.body, input);
    }

    #[test]
    fn extracts_h1_as_title_and_strips_it() {
        let (title, body) = extract_h1_title("# My Title\n\nBody here.");
        assert_eq!(title.as_deref(), Some("My Title"));
        assert_eq!(body.trim(), "Body here.");
    }

    #[test]
    fn resolves_plain_date_to_midnight_utc() {
        let resolved = resolve_date(Some("2024-01-15"), std::time::SystemTime::UNIX_EPOCH);
        assert_eq!(resolved, "2024-01-15T00:00:00.000Z");
    }
}
