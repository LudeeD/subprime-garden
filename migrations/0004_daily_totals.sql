-- Site-wide views/uniques per day. daily_stats is per path, and summing its
-- per-path uniques counts a visitor once for every page they read.
CREATE TABLE daily_totals (
  day      TEXT PRIMARY KEY,
  views    INTEGER NOT NULL DEFAULT 0,
  uniques  INTEGER NOT NULL DEFAULT 0
);

-- Best available backfill for days whose raw pageviews are already purged;
-- the next rollup overwrites the days still inside the retention window.
INSERT INTO daily_totals (day, views, uniques)
  SELECT day, SUM(views), SUM(uniques) FROM daily_stats GROUP BY day;
