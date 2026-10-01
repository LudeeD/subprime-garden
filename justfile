# Runs a dev instance out of the repo root at http://127.0.0.1:8080 (admin at
# /admin). The first run prompts for an admin password, writes garden.toml,
# and imports demo/posts; later runs reuse garden.toml and ./data. Every run
# overwrites ./templates and ./static from src/render/default_* so stock theme
# edits show up on restart. `rm -rf garden.toml data` to start over.
dev:
    #!/usr/bin/env bash
    set -euo pipefail
    cargo build
    bin=target/debug/subprime-garden
    if [ ! -f garden.toml ]; then
        SUBPRIME_SITE__TITLE="subprime garden (dev)" \
        SUBPRIME_SITE__DESCRIPTION="A dev instance with demo posts." \
        SUBPRIME_SITE__AUTHOR="$(git config user.name)" \
        SUBPRIME_SITE__BASE_URL="http://localhost:8080" \
        SUBPRIME_AUTH__USERNAME="owner" \
            "$bin" init .
        # init has no prompt for taxonomies; the demo posts also use "series".
        sed -i 's/^taxonomies = \["tags"\]$/taxonomies = ["tags", "series"]/' garden.toml
        "$bin" import demo/posts --published
    fi
    "$bin" theme . --force > /dev/null
    exec "$bin" serve

# Tags and pushes a release named <year>.<month>.<day>-<8-char commit hash>,
# e.g. 2026.08.08-b9479282 — matches the pattern .github/workflows/docker.yml
# triggers on, so pushing it kicks off the docker build+publish. `origin` is
# configured to push to both GitHub and the sourcehut mirror.
release:
    #!/usr/bin/env bash
    set -euo pipefail
    if [ -n "$(git status --porcelain)" ]; then
        echo "working tree is dirty — commit or stash before tagging" >&2
        exit 1
    fi
    name="$(date +%Y.%m.%d)-$(git rev-parse --short=8 HEAD)"
    git tag -a "$name" -m "release $name"
    git push origin "$name"
    echo "released $name"
