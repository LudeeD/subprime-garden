-- content_hash was written on every save but never read: page ETags are
-- computed from the rendered page, not from this column.
ALTER TABLE posts DROP COLUMN content_hash;
