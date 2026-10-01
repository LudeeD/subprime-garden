---
title: SQLite is enough
date: 2026-09-20
tags: [rust, sqlite]
taxonomies:
  series: [Building the garden]
description: Why a single-file database is the right fit for a single-user blog.
---

One writer, a handful of readers, and a database that is just a file you can `cp`.

| Concern   | Answer                  |
|-----------|-------------------------|
| Backups   | copy one file           |
| Scaling   | you are one person      |
| Ops       | there are none          |

```sql
SELECT title, published_at FROM posts WHERE status = 'published' ORDER BY published_at DESC;
```
