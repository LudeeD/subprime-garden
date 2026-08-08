-- Generalizes tags into taxonomies: a taxonomy is a named grouping of terms
-- (e.g. "tags", "series"), configured in garden.toml. Existing tags become
-- the "tags" taxonomy so nothing changes for sites that don't add more.
CREATE TABLE taxonomy_terms (
  id        INTEGER PRIMARY KEY,
  taxonomy  TEXT NOT NULL,
  name      TEXT NOT NULL,
  slug      TEXT NOT NULL,
  UNIQUE(taxonomy, name),
  UNIQUE(taxonomy, slug)
);

CREATE TABLE post_taxonomy_terms (
  post_id INTEGER NOT NULL REFERENCES posts(id) ON DELETE CASCADE,
  term_id INTEGER NOT NULL REFERENCES taxonomy_terms(id) ON DELETE CASCADE,
  PRIMARY KEY (post_id, term_id)
);
CREATE INDEX idx_post_taxonomy_terms_term ON post_taxonomy_terms(term_id);

INSERT INTO taxonomy_terms (id, taxonomy, name, slug)
  SELECT id, 'tags', name, slug FROM tags;

INSERT INTO post_taxonomy_terms (post_id, term_id)
  SELECT post_id, tag_id FROM post_tags;

DROP TABLE post_tags;
DROP TABLE tags;
