#!/usr/bin/env bash
set -euo pipefail

echo "RepoScope Desktop packages do not include a but CLI or global symlink." >&2
echo "This upstream helper is intentionally disabled." >&2
exit 1
