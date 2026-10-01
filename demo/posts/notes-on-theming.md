+++
title = "Notes on theming"
date = 2026-09-27
description = "Templates and CSS live next to the binary, so restyling never needs a rebuild."

[taxonomies]
tags = ["meta", "design"]
series = ["Building the garden"]
+++

`subprime-garden init` scaffolds `./templates` and `./static` from the stock theme. Edit them in place and restart — no recompile.

1. Run `subprime-garden theme` to refresh the stock files
2. Tweak `static/` CSS
3. Reload
