#!/bin/sh
# Every gate CI runs, stopping at the first failure. Run before committing.
set -eu
cd "$(dirname "$0")/.."
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
ABSHARE_REQUIRE_LANDLOCK=1 cargo test --workspace
(cd web/app && npm run check && npm test && npm run build)
echo "check.sh: all gates passed"
