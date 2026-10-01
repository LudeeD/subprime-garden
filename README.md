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
| `export <dir>` | Write every post and page out as markdown with YAML frontmatter. |
| `rerender` | Rebuild stored HTML for every post, after changing `[markdown]` options. |
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

Set `server.trust_proxy_headers = true` only behind a single reverse proxy
you control: the client IP is then taken from the last `X-Forwarded-For`
entry.

## Writing

- Posts are markdown, saved as drafts until published. A post of kind *page*
  lives at `/<slug>` like a post but stays out of the index, archive and
  feeds.
- `site.taxonomies` lists the groupings posts can have (default: `tags`).
  Each gets a `/<name>` index and `/<name>/<term>` pages.
- Images are uploaded under `/admin/media` (jpeg, png, webp, gif, svg). A
  bare image path in markdown, `![alt](photo.png)`, resolves to `/media/`.
  Images wider than 1600px also get a downscaled webp variant.

## Theming

`serve` loads templates from `./templates` and assets from `./static`,
relative to its working directory, at startup. They are yours to edit;
restart to apply. The admin UI is built into the binary and not themeable.

Templates are [minijinja](https://docs.rs/minijinja) (Jinja2 syntax). Every
template gets `site` (`title`, `description`, `base_url`, `author`,
`taxonomies`) and `pages`, the published pages keyed by slug — so
`{{ pages.about.html|safe }}` embeds the page with slug `about` anywhere.

| Template | Extra context |
|---|---|
| `index.html` | `posts`: every published post, newest first. The stock theme lists `posts[:10]`; change the slice to taste. |
| `archive.html` | `posts` |
| `post.html` | `post` |
| `taxonomy_index.html` | `taxonomy`, `taxonomy_label`, `terms` (`name`, `slug`, `count`) |
| `taxonomy_term.html` | `taxonomy`, `taxonomy_label`, `term_name`, `posts` |

A post has `slug`, `title`, `html`, `excerpt`, `published_at`,
`published_at_human`, and (on `post.html` only) `taxonomies`.

After upgrading the binary, run `subprime-garden theme` to add any stock
template your theme doesn't have yet; the server refuses to start if a
required one is missing.

Code blocks are highlighted when a post is saved, with a fixed dark theme
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
