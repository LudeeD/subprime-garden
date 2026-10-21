# subprime-garden

A self-hosted blog for exactly one person.

One binary, one SQLite file, markdown posts written in a small admin UI, a
theme you own as plain template files, and privacy-friendly analytics.

## Quickstart

### Docker

Run from `deployment/docker`. The container runs as UID 1000 (non-root), so
the directories it writes to must be writable by that user.

```sh
mkdir -p data import templates static && touch garden.toml
docker compose run --rm garden init    # prompts for site details and a password
docker compose up -d
```

The site is on <http://127.0.0.1:8080>, the admin at `/admin`. For automatic
HTTPS, copy `.env.example` to `.env`, set `SUBPRIME_DOMAIN`, and use
`compose.caddy.yaml` in place of `compose.yaml` in the commands above.

### Binary + systemd

```sh
subprime-garden init /etc/subprime-garden
cp deployment/systemd/subprime-garden.service /etc/systemd/system/
systemctl enable --now subprime-garden
```

The unit runs as the `subprime-garden` user (create it first), reads its config and theme from
`/etc/subprime-garden`, and keeps the database and media in
`/var/lib/subprime-garden`.

## Commands

| Command | What it does |
|---|---|
| `serve` | Run the HTTP server. |
| `init [dir]` | Scaffold `templates/` and `static/`, and walk `garden.toml` to a complete config. `--reset-password` sets a new admin password and logs out existing sessions; `--force` starts over. |
| `theme [dir]` | Scaffold the stock theme only. Writes just the files that are missing; `--force` overwrites all of them. |
| `import <dir>` | Import a directory of markdown files (YAML or TOML frontmatter, as written by Zola, Hugo or Jekyll). `--published`, `--force`, `--dry-run`. |
| `export <dir>` | Write every item out as markdown with YAML frontmatter. |
| `rerender` | Rebuild stored HTML for every item, after changing `[markdown]` options. |
| `migrate` | Apply pending database migrations (`serve` does this on startup too). |

`import` and `rerender` write to the database directly: restart a running
server afterwards to see the changes.

## Configuration

Config is read from `--config <path>`, else `$SUBPRIME_CONFIG`, else
`./garden.toml` if it exists. See `garden.toml.example` for every key.

Any key can also be set by environment variable, and environment wins over
the file — useful for keeping secrets out of it:
`SUBPRIME_<SECTION>__<KEY>`, e.g. `SUBPRIME_SITE__TITLE`,
`SUBPRIME_AUTH__PASSWORD_HASH`, `SUBPRIME_AUTH__SESSION_SECRET`.

Collections are arrays of tables, so by environment they are one TOML
value: `SUBPRIME_COLLECTIONS='[{name="posts", in_feed=true}, {name="pages"}]'`.

Set `server.trust_proxy_headers = true` only behind a single reverse proxy
you control: the client IP is then taken from the last `X-Forwarded-For`
entry.

## Writing

Content comes in **collections**, declared in `garden.toml`. `posts` and
`pages` are just the two that `init` sets up; add as many as you like:

```toml
[[collections]]
name = "posts"
in_feed = true            # on the home page and in the feeds (default: false)

[[collections]]
name = "pages"

[[collections]]
name = "books"
sort = "finished desc"    # default: "published_at desc"; also title or any field
fields.author   = "text"
fields.rating   = "int"
fields.finished = "date"
fields.cover    = "media"
fields.shelf    = { choice = ["read", "reading", "want-to-read"] }
```

- Every item lives at `/<slug>`, whatever its collection; slugs are unique
  across collections. Every collection gets a list page at `/<name>`, so a
  collection name can't also be a slug, a taxonomy or a built-in route.
- Field types: `text`, `int`, `date` (`YYYY-MM-DD`), `url` (absolute http or
  https), `media` (the filename of an upload), and `choice`. Every field is
  optional. A value that doesn't fit the schema, or a field the collection
  doesn't declare, is an error — in the admin, on import, and at startup for
  what's already stored, which names the items to fix.
- `site.taxonomies` lists the groupings items can have (default: `tags`).
  Each gets a `/<name>` index and `/<name>/<term>` pages, listing items from
  every collection. If you'd browse it across collections, make it a
  taxonomy; if it only describes items in one collection (a book's shelf),
  make it a field.
- Items are markdown, saved as drafts until published. Frontmatter for
  `import` (and what `export` writes):

  ```yaml
  ---
  title: The Dispossessed
  date: 2026-09-14
  collection: books          # default: posts
  fields:
    author: Ursula K. Le Guin
    rating: 5
  taxonomies:
    tags: [anarchism]
  ---
  ```

  Media fields hold filenames only: restore the media directory before
  importing items that use them.
- Images are uploaded under `/admin/media` (jpeg, png, webp, gif, svg). A
  bare image path in markdown, `![alt](photo.png)`, resolves to `/media/`.
  Images wider than 1600px also get a downscaled webp variant. An upload a
  media field uses can't be deleted.

### Upgrading from posts and pages

Before collections, every item was a post or a page. The database migrates
itself on the next start (posts go to `posts`, pages to `pages`), but the
config has to declare those collections: run `subprime-garden init` once,
which adds them to an existing `garden.toml` and touches nothing else. Then
run `subprime-garden theme --force` (or merge by hand), since the stock
templates changed shape: `post.html` and `archive.html` are gone, `/archive`
is now `/posts`, and the `/tag/<term>` redirect is gone.

## Theming

`serve` loads templates from `./templates` and assets from `./static`,
relative to its working directory, at startup. They are yours to edit;
restart to apply. The admin UI is built into the binary and not themeable.

Templates are [minijinja](https://docs.rs/minijinja) (Jinja2 syntax), and
strict: using a variable that doesn't exist is an error, not an empty string.
Every template gets `site` (`title`, `description`, `base_url`, `author`,
`taxonomies`, `collections`) and the function `get_item(slug)`, which returns
the published item with that slug or `none` — so
`{% set about = get_item("about") %}{% if about %}{{ about.html|safe }}{% endif %}`
embeds it anywhere.

Templates are looked up by convention, falling back to a stock one:

| Page | Looks for | Falls back to | Extra context |
|---|---|---|---|
| `/` | `index.html` | | `items`: published items of the `in_feed` collections, newest first. The stock theme lists `items[:10]`. |
| `/books` | `books.html` | `collection.html` | `collection` (`name`, `label`), `items` in the collection's sort order |
| `/the-dispossessed` | `books_entry.html` | `entry.html` | `item` |
| an item in another list | `books_card.html` | `card.html` | `item` |
| `/tags` | `taxonomy_index.html` | | `taxonomy`, `taxonomy_label`, `terms` (`name`, `slug`, `count`) |
| `/tags/anarchism` | `taxonomy_term.html` | | `taxonomy`, `taxonomy_label`, `term_name`, `items` |

Cards are chosen by the list template itself:
`{% include [item.collection ~ "_card.html", "card.html"] %}`.

An item looks the same everywhere: `slug`, `title`, `html`, `excerpt`,
`collection`, `published_at`, `published_at_human`, `updated_at`,
`taxonomies` (a list of `taxonomy` + `terms`), and `fields`, which has
every field the collection declares — `none` when the item leaves it empty,
and `{url, width, height}` for a media field.

After upgrading the binary, run `subprime-garden theme` to add any stock
template your theme doesn't have yet; the server refuses to start if a
required one is missing.

Code blocks are highlighted when an item is saved, with a fixed dark theme
(`base16-ocean.dark`, set in `src/content/markdown.rs`). Changing it means
rebuilding the binary and running `rerender`.

## Analytics

No cookies, no third parties. A visitor is a hash of IP and user agent with
a salt that changes daily, so visitors can't be linked across days. Raw
pageviews are kept for `analytics.raw_retention_days`, then only per-day
totals remain. Feed reads are counted in memory and reset on restart.

## Development

```sh
just dev      # dev instance with demo posts on http://127.0.0.1:8080
cargo test
```

## License

MIT — see `LICENSE`.
