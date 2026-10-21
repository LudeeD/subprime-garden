-- Replaces the fixed post/page `kind` with collections declared in
-- garden.toml. Existing posts and pages land in the stock `posts` and `pages`
-- collections; `fields` holds each item's typed collection fields as JSON.
ALTER TABLE posts ADD COLUMN collection TEXT NOT NULL DEFAULT 'posts';
ALTER TABLE posts ADD COLUMN fields TEXT NOT NULL DEFAULT '{}';
UPDATE posts SET collection = CASE kind WHEN 'page' THEN 'pages' ELSE 'posts' END;

DROP INDEX idx_posts_kind;
ALTER TABLE posts DROP COLUMN kind;
CREATE INDEX idx_posts_collection ON posts(collection, status, published_at DESC);
