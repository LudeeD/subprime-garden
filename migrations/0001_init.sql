CREATE TABLE posts (
  id            INTEGER PRIMARY KEY,
  slug          TEXT NOT NULL UNIQUE,
  title         TEXT NOT NULL,
  markdown      TEXT NOT NULL,
  html          TEXT NOT NULL,
  excerpt       TEXT NOT NULL DEFAULT '',
  content_hash  TEXT NOT NULL,
  status        TEXT NOT NULL CHECK(status IN ('draft','published')) DEFAULT 'draft',
  kind          TEXT NOT NULL CHECK(kind IN ('post','page')) DEFAULT 'post',
  created_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  updated_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  published_at  TEXT
);
CREATE INDEX idx_posts_status_published ON posts(status, published_at DESC);
CREATE INDEX idx_posts_kind ON posts(kind);

CREATE TABLE tags (
  id    INTEGER PRIMARY KEY,
  name  TEXT NOT NULL UNIQUE,
  slug  TEXT NOT NULL UNIQUE
);

CREATE TABLE post_tags (
  post_id INTEGER NOT NULL REFERENCES posts(id) ON DELETE CASCADE,
  tag_id  INTEGER NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
  PRIMARY KEY (post_id, tag_id)
);
CREATE INDEX idx_post_tags_tag ON post_tags(tag_id);

CREATE TABLE media (
  id                INTEGER PRIMARY KEY,
  filename          TEXT NOT NULL UNIQUE,
  original_name     TEXT NOT NULL,
  mime              TEXT NOT NULL,
  bytes             INTEGER NOT NULL,
  width             INTEGER,
  height            INTEGER,
  variant_filename  TEXT,
  created_at        TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

CREATE TABLE pageviews (
  id             INTEGER PRIMARY KEY,
  path           TEXT NOT NULL,
  post_id        INTEGER REFERENCES posts(id) ON DELETE SET NULL,
  referrer_host  TEXT,
  visitor_hash   TEXT NOT NULL,
  created_at     TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
CREATE INDEX idx_pageviews_created ON pageviews(created_at);
CREATE INDEX idx_pageviews_path ON pageviews(path);

CREATE TABLE daily_stats (
  day      TEXT NOT NULL,
  path     TEXT NOT NULL,
  post_id  INTEGER REFERENCES posts(id) ON DELETE SET NULL,
  views    INTEGER NOT NULL DEFAULT 0,
  uniques  INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (day, path)
);

CREATE TABLE salts (
  day   TEXT PRIMARY KEY,
  salt  BLOB NOT NULL
);
