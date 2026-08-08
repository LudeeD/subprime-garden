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
