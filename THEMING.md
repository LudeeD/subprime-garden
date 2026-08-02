# Theming subprime-garden

Askama compiles templates into the binary. There is no runtime template
engine and no theme-switcher — **forking the look of this blog means editing
files under `templates/` and `static/style.css`, then `cargo build --release`
(or letting your fork's CI do it — see the README's fork-and-push loop).**

This document is the contract: every template file, every field its Rust
context struct hands it, and every CSS class/variable the stock stylesheet
defines. If you replace `templates/` and `static/style.css` wholesale with
your own, this is everything you need to know to keep the app working.

Context struct source: `src/render/mod.rs`. Template files: `templates/`.

## Shared context: `SiteView`

Passed as `site` to every public and admin template.

| Field | Type | Notes |
|---|---|---|
| `title` | String | `[site] title` from config |
| `description` | String | `[site] description` |
| `base_url` | String | `[site] base_url`, no trailing slash assumed |
| `author` | String | `[site] author` |

## Public templates

### `templates/base.html`
Base layout. Blocks: `title` (default: `{{ site.title }}`), `content`.
Renders the header (site title + nav linking `/archive`) and footer
(`{{ site.author }}`). Links `/static/style.css` and `/feed.xml`.

### `templates/index.html` — `IndexTemplate`
Extends `base.html`. The paginated post list at `/` and `/page/:n`.

| Field | Type |
|---|---|
| `site` | `SiteView` |
| `posts` | `Vec<PostView>` |
| `pagination` | `PaginationView` |

Includes `templates/partials/pagination.html`.

### `templates/post.html` — `PostTemplate`
Extends `base.html`. A single post or page at `/:slug`, and admin
preview/publish flows reuse this same template.

| Field | Type |
|---|---|
| `site` | `SiteView` |
| `post` | `PostView` (tags populated here, unlike list contexts) |

`post.html` is output via the `|safe` filter — it's pre-rendered, sanitized
HTML from the markdown pipeline, not escaped again.

### `templates/archive.html` — `ArchiveTemplate`
Extends `base.html`. Full chronological list at `/archive`, no pagination.

| Field | Type |
|---|---|
| `site` | `SiteView` |
| `posts` | `Vec<PostView>` (tags left empty — see `PostView` below) |

### `templates/tag.html` — `TagTemplate`
Extends `base.html`. Posts for one tag at `/tag/:slug`.

| Field | Type |
|---|---|
| `site` | `SiteView` |
| `tag_name` | String |
| `posts` | `Vec<PostView>` |

### `templates/partials/pagination.html` — `PaginationView`
Included by `index.html`, not routed directly.

| Field | Type |
|---|---|
| `has_prev` / `has_next` | bool |
| `prev_url` / `next_url` | String |

### `PostView` (used by all four templates above)

| Field | Type | Notes |
|---|---|---|
| `slug` | String | |
| `title` | String | |
| `html` | String | Pre-rendered; render with `\|safe` |
| `excerpt` | String | Auto-derived from the first paragraph, or frontmatter `description` on imported posts |
| `published_at` | String | ISO 8601, for `<time datetime>` |
| `published_at_human` | String | e.g. "August 2, 2026" |
| `tags` | `Vec<TagView>` | Only populated on the post-detail page (`with_tags`); empty in index/archive/tag listing rows |

`TagView`: `{ name: String, slug: String }`.

Feeds (`/feed.xml`, `/rss.xml`) and `/sitemap.xml` are **not** Askama
templates — they're hand-built, escaped XML strings in `src/render/feeds.rs`,
since Askama's default escaper doesn't cover XML. If you need to reskin the
feed output specifically, edit that file, not `templates/`.

## Admin templates

All admin pages share the same `static/style.css` as the public site — no
separate admin stylesheet.

### `templates/admin/base.html`
Admin chrome: nav (posts / new / media / analytics / view site / logout
form). Blocks: `title`, `content`. Every admin template below extends this
**except** `login.html`, which has no session yet to build the logout form
from and is a standalone `<html>` document.

### `templates/admin/login.html` — `LoginTemplate`
| Field | Type |
|---|---|
| `site` | `SiteView` |
| `csrf_token` | String — this form's own short-lived CSRF cookie, not a session |
| `error` | `Option<String>` |

### `templates/admin/dashboard.html` — `DashboardTemplate`
`GET /admin`.

| Field | Type |
|---|---|
| `site` | `SiteView` |
| `csrf_token` | String — the logged-in session's CSRF token |
| `recent_posts` | `Vec<AdminPostRow>` |
| `published_count` / `draft_count` | i64 |
| `views_7d` / `uniques_7d` | i64 |

### `templates/admin/posts_list.html` — `PostsListTemplate`
`GET /admin/posts[?status=draft\|published]`.

| Field | Type |
|---|---|
| `site` | `SiteView` |
| `csrf_token` | String |
| `posts` | `Vec<AdminPostRow>` |

`AdminPostRow`: `{ id: i64, slug: String, title: String, status: String, kind: String, updated_at_human: String }`.

### `templates/admin/post_edit.html` — `PostEditTemplate`
`GET /admin/posts/new`, `GET /admin/posts/:id/edit`. Handles both create and
edit — `is_new` picks the form action. Contains the only non-trivial inline
`<script>` in the app: a 30-second autosave poll and a delete confirmation,
both admin-only (the public site ships zero JavaScript).

| Field | Type |
|---|---|
| `site` | `SiteView` |
| `csrf_token` | String |
| `is_new` | bool |
| `id` | i64 (0 when `is_new`) |
| `slug`, `title`, `markdown`, `kind`, `status` | String |
| `saved` | bool — shows the "Saved." flash after a redirect |
| `tags` | String — comma-separated, pre-filled from the post's current tags |

### `templates/admin/media.html` — `MediaGridTemplate`
`GET /admin/media`.

| Field | Type |
|---|---|
| `site` | `SiteView` |
| `csrf_token` | String |
| `items` | `Vec<AdminMediaRow>` |

`AdminMediaRow`: `{ id, filename, original_name, width: Option<i64>, height: Option<i64>, markdown_snippet: String }`. The "copy markdown" button's tiny inline script reads `data-snippet` — no templating on the client.

### `templates/admin/analytics.html` — `AnalyticsTemplate`
`GET /admin/analytics`.

| Field | Type |
|---|---|
| `site` | `SiteView` |
| `csrf_token` | String |
| `total_views` | i64 |
| `views_7d`/`30d`/`90d`, `uniques_7d`/`30d`/`90d` | i64 |
| `sparkline_7d`/`30d`/`90d` | String — pre-rendered inline `<svg>`, output with `\|safe` |
| `top_posts` / `top_referrers` | `Vec<CountRow>` |
| `dropped_events` | u64 — pageviews dropped because the write buffer was full |
| `feed_hits` | u64 — since last restart (not persisted) |

`CountRow`: `{ label: String, count: i64 }`.

## CSS reference (`static/style.css`, one file, no build step)

### Custom properties (light values; overridden under `prefers-color-scheme: dark`)

`--bg`, `--fg`, `--muted`, `--link`, `--border`, `--code-bg`.

### Structural classes

| Class | Where |
|---|---|
| `.site-header`, `.site-title`, `.site-footer` | base layout, both public and admin |
| `.post-summary` | one entry in index/archive/tag listings |
| `.post-body` | the rendered post HTML wrapper — also styles `img`, `pre`, `code`, `blockquote`, `table` inside it |
| `.tags` | tag chip line under a post |
| `.pagination` | prev/next links on the index |
| `.archive-list` | the `/archive` list |

### Admin-only classes

| Class | Where |
|---|---|
| `.link-button` | a `<button>` styled as an underlined link (logout, publish, delete, …) |
| `.inline-form` | a one-button `<form>` that needs to sit inline with text |
| `.flash`, `.flash-error` | "Saved."/login-error banners |
| `.admin-post-list`, `.admin-post-table` | dashboard recent list / posts table |
| `.post-meta` | secondary metadata text (status, date, view counts) |
| `.login-page` | narrow centered login form wrapper |
| `.media-grid`, `.media-tile` | media grid |
| `.analytics-grid`, `.sparkline` | analytics page layout and the inline SVG sparklines (`stroke="currentColor"`, so they follow the surrounding text color / theme automatically) |

Syntax-highlighted code blocks are baked into stored HTML at save time via
`syntect` using a **fixed dark theme** (`base16-ocean.dark`, set in
`src/content/markdown.rs`), independent of the site's own light/dark mode —
see the comment there if you want to change it (requires `subprime-garden
rerender` afterward).
