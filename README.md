# subprime-garden

A self-hosted blog for exactly one person. Bear-Blog-plain on the reading
side (zero JavaScript, one small stylesheet), Zola-ish on the operator side
(real CLI, importable content, a Docker Compose quickstart).

## Quickstart (Docker Compose)

You need a server with Docker installed and, if you want your own domain
with HTTPS, DNS pointed at it.

```sh
git clone <your fork's URL>
cd subprime-garden
cp .env.example .env
```

Generate a password hash and a session secret, and put them in `.env`:

```sh
docker compose run --rm garden hash-password   # -> GARDEN_PASSWORD_HASH
openssl rand -base64 48                        # -> GARDEN_SESSION_SECRET
```

Edit `compose.yaml` (or `compose.caddy.yaml`) to point `SUBPRIME_SITE__BASE_URL`
at your real domain, then:

```sh
docker compose up -d
```

That's it — migrations run automatically on first boot. Log in at
`/admin/login` with the username from `compose.yaml` (`owner` by default)
and the password you hashed above.

Two Compose files are provided:

- **`compose.yaml`** — just the app, listening on `127.0.0.1:8080`. Use this
  if you already run a reverse proxy (nginx, Caddy, Traefik) on the host.
- **`compose.caddy.yaml`** — the app plus a Caddy sidecar that gets you a
  Let's Encrypt certificate automatically. Set `SUBPRIME_DOMAIN` in `.env`
  and run `docker compose -f compose.caddy.yaml up -d` instead. Nothing else
  to configure — Caddy handles the ACME challenge and renewal.

Updating either way:

```sh
docker compose pull && docker compose up -d
```

### The fork-and-CI loop (read this before you touch `templates/`)

Askama compiles templates into the binary at build time — there is no
runtime theme engine. That means **the image `compose.yaml` pulls must be
built from your own fork**, not from some upstream registry, or your first
`docker compose pull` would silently overwrite your edits with the stock
theme.

So the update loop is:

1. Fork this repo.
2. Edit `templates/` and `static/style.css` however you like.
3. Push to `main` (or push a tag).
4. `.github/workflows/docker.yml` builds a multi-arch image and pushes it to
   `ghcr.io/<your-username>/subprime-garden`.
5. Point `compose.yaml`'s `image:` at that (it already says
   `ghcr.io/OWNER/subprime-garden:latest` — replace `OWNER`).
6. On the server: `docker compose pull && docker compose up -d`.

No GitHub remote is required for local development or for the bare-binary
path below — only for step 4, the automated build.

See `THEMING.md` for the full template/context/CSS reference before you
start editing.

## Running without Docker

```sh
cargo build --release
sudo cp target/release/subprime-garden /usr/local/bin/
sudo useradd --system --home /var/lib/subprime-garden subprime-garden
sudo mkdir -p /var/lib/subprime-garden /etc/subprime-garden
sudo chown subprime-garden:subprime-garden /var/lib/subprime-garden
subprime-garden hash-password   # paste the result into garden.toml below
```

Copy `garden.toml.example` to `/etc/subprime-garden/garden.toml`, fill it
in (`database`/`media_dir` under `/var/lib/subprime-garden`), then:

```sh
sudo cp systemd/subprime-garden.service /etc/systemd/system/
sudo systemctl enable --now subprime-garden
```

`bind` defaults to `127.0.0.1:8080` outside a container — put a reverse
proxy (Caddy, nginx) in front of it for TLS.

## CLI

Everything is a subcommand of the same binary and operates directly on the
SQLite file — there's no client/server split, no API tokens. Run it on the
box (or via `docker compose run --rm garden <subcommand>`):

| Command | What it does |
|---|---|
| `serve` | Run the HTTP server. |
| `hash-password` | Prompt for a password, print an argon2 hash for the config. |
| `migrate` | Apply pending migrations and exit. |
| `import <DIR> [--published] [--force] [--dry-run]` | Walk a directory of `.md` files (Zola/Hugo/Jekyll frontmatter accepted) and load them. Idempotent by slug — re-running skips posts that already exist unless `--force`. |
| `export <DIR>` | Write every post/page back out as `.md` with YAML frontmatter — round-trips losslessly. |
| `rerender` | Rebuild stored HTML for every post. Needed after changing `[markdown]` options or the syntax-highlighting theme. |
| `healthcheck` | Hits `/healthz` over a raw TCP connection; used as the container `HEALTHCHECK` since the runtime image has no shell or curl. |

There's no `backup` subcommand on purpose —
`sqlite3 /data/garden.db ".backup out.db"` plus a `tar` of the media
directory already does this, better than a subcommand would, and works the
same with or without Docker.

## Configuration

One TOML file, path via `--config` or `$SUBPRIME_CONFIG` — see
`garden.toml.example` for a complete annotated copy. **Every field is also
settable via a `SUBPRIME_<SECTION>__<FIELD>` environment variable** (double
underscore between section and field), which always wins over the file. A
Compose deployment needs no config file at all — see `compose.yaml`, which
sets everything through `environment:`.

| Section.field | Env var | Default | Notes |
|---|---|---|---|
| `site.title` | `SUBPRIME_SITE__TITLE` | `subprime garden` | |
| `site.description` | `SUBPRIME_SITE__DESCRIPTION` | *(empty)* | |
| `site.base_url` | `SUBPRIME_SITE__BASE_URL` | `http://localhost:8080` | No trailing slash. Used in feeds, sitemap, `og`-style absolute links. |
| `site.author` | `SUBPRIME_SITE__AUTHOR` | *(empty)* | |
| `site.timezone` | `SUBPRIME_SITE__TIMEZONE` | `UTC` | Reserved for display formatting. |
| `site.posts_per_page` | `SUBPRIME_SITE__POSTS_PER_PAGE` | `20` | |
| `server.bind` | `SUBPRIME_SERVER__BIND` | `127.0.0.1:8080` | **Must** be `0.0.0.0:8080` in a container — the app warns loudly at startup if it detects a container and is still bound to loopback. |
| `server.database` | `SUBPRIME_SERVER__DATABASE` | `./data/garden.db` | |
| `server.media_dir` | `SUBPRIME_SERVER__MEDIA_DIR` | `./data/media` | |
| `server.trust_proxy_headers` | `SUBPRIME_SERVER__TRUST_PROXY_HEADERS` | `false` | Only enable behind a reverse proxy you control — otherwise `X-Forwarded-For` is a trivial spoof for rate-limit bypass and analytics corruption. |
| `auth.username` | `SUBPRIME_AUTH__USERNAME` | *(required)* | |
| `auth.password_hash` | `SUBPRIME_AUTH__PASSWORD_HASH` | *(required)* | Must be an argon2 hash (`subprime-garden hash-password`) — a plaintext value here is a startup error, not a warning. |
| `auth.session_secret` | `SUBPRIME_AUTH__SESSION_SECRET` | *(required)* | At least 32 bytes; stretched internally via HKDF, so any random string that long works. |
| `auth.session_ttl_days` | `SUBPRIME_AUTH__SESSION_TTL_DAYS` | `30` | |
| `analytics.enabled` | `SUBPRIME_ANALYTICS__ENABLED` | `true` | |
| `analytics.raw_retention_days` | `SUBPRIME_ANALYTICS__RAW_RETENTION_DAYS` | `30` | Raw per-visit rows (and their daily salts) older than this are deleted nightly; aggregate `daily_stats` are kept forever. |
| `analytics.ignore_bots` | `SUBPRIME_ANALYTICS__IGNORE_BOTS` | `true` | Substring match against a built-in bot user-agent list. |
| `markdown.syntax_highlighting` | `SUBPRIME_MARKDOWN__SYNTAX_HIGHLIGHTING` | `true` | Baked into stored HTML at save time — run `rerender` after changing this. |
| `markdown.smart_punctuation` | `SUBPRIME_MARKDOWN__SMART_PUNCTUATION` | `true` | |
| `markdown.footnotes` | `SUBPRIME_MARKDOWN__FOOTNOTES` | `true` | |
| `media.max_upload_bytes` | `SUBPRIME_MEDIA__MAX_UPLOAD_BYTES` | `10485760` (10 MiB) | |

The config fails loudly and specifically on startup if it's missing,
malformed, has a plaintext password where a hash belongs, or a session
secret under 32 bytes — it won't silently start insecurely.

## Architecture notes (for anyone extending this)

- **SQLite via `rusqlite` + `r2d2`, not `sqlx`.** Single-writer workload, so
  there's nothing async buys here; a small blocking pool run through
  `spawn_blocking` keeps the request path non-blocking without adopting a
  whole async DB runtime. See `src/db/mod.rs`.
- **Sessions are encrypted cookies (`axum-extra`'s `PrivateCookieJar`), not a
  DB-backed session store.** There's exactly one identity; a cookie holding
  `{issued_at, csrf}` is simpler than a `sessions` table plus cleanup task.
  Revocation story: rotate `auth.session_secret` and every cookie dies at
  once.
- **Markdown is rendered exactly once, at write time** (`src/content/markdown.rs`).
  Requests never touch `pulldown-cmark` or `syntect`. Heading anchors, image
  `/media/` rewriting, lazy-loading attributes, and `<picture>`/webp-variant
  swapping all happen in that same pass.
- **Full-page cache** (`src/render/cache.rs`) is a plain
  `RwLock<HashMap<path, CachedPage>>` — no TTL, no LRU. Any post/page
  mutation clears the whole thing; that's simpler and correct for a
  single-writer blog where writes are rare relative to reads, and avoids
  needing a smarter cache library (`moka`) for an eviction policy this
  project doesn't need.
- **Analytics never block a response.** The pageview middleware
  (`src/web/middleware.rs`) hashes the visitor (one cached-salt read, no
  I/O) and hands the event to a bounded channel via `try_send` — a full
  channel just drops the event and increments a counter. A background task
  batches inserts every 500ms/100 rows; graceful shutdown drains it (see
  `analytics_writer.await` in `src/main.rs`).

## Performance

Two things make cached responses fast: the full-page `RwLock<HashMap>` cache
(§ above) and `ETag`/`If-None-Match` handling that returns `304` without
re-rendering or even cloning the cached body (`Arc<str>` clone only).

Real p50/p99 numbers for cached `/` and cached `/:slug` under load aren't
included in this copy of the README — they need to be measured against a
running instance, not asserted. To measure them yourself:

```sh
cargo build --release
./target/release/subprime-garden serve &

# first request populates the cache (or startup warming already did);
# everything after is the cached path
hey -n 2000 -c 50 http://127.0.0.1:8080/
hey -n 2000 -c 50 http://127.0.0.1:8080/your-post-slug
```

(`hey` or `wrk` both work; install whichever you have.) Release binary size
on this build: **11 MB** (`target/release/subprime-garden`, stripped). The
Docker image size target is "well under 30 MB" (distroless base + static
musl binary, no shell, no package manager) — check `docker images` against
your own build for the current number, since it moves as dependencies do.

## Non-goals

No user registration, roles, comments, JS framework/bundler, plugin system,
webmentions, email newsletter, cloud storage adapters, or third-party REST
API. If a future change needs a dependency to support one of these, that's
a sign to stop and reconsider, not to add the dependency.
