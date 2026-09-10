#!/usr/bin/env bash
set -euo pipefail

if [ "${CI:-}" = "true" ]; then
  echo "CI environment detected, expecting frontend build to be downloaded."
else
  fe_mode=${1:?First argument must be the mode to build the frontend with}
  echo "Assuming local invocation, building frontend in $fe_mode mode"
  pnpm build:desktop -- --mode "$fe_mode"
fi

set -x
# RepoScope Desktop is a self-contained desktop application.  The upstream
# build used this hook to compile and inject `but` and Git Askpass helper
# binaries as Tauri `externalBin` entries.  Those helpers can initiate remote
# Git traffic and are intentionally not part of the offline product.
# Local GitButler operations use the in-process Rust APIs instead.
