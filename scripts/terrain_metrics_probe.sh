#!/usr/bin/env bash
set -euo pipefail

# Run the terrain metrics probe (`--terrain-metrics`) in release mode.
# Defaults to a radius of 6 (≈1100 chunks); pass `--terrain-metrics-radius` to override.
# Additional CLI arguments are forwarded to the underlying `cargo run` call.

SCRIPT_DIR=$(cd -- "$(dirname "${BASH_SOURCE[0]}")" && pwd)
REPO_ROOT=$(cd -- "${SCRIPT_DIR}/.." && pwd)

cmd=(cargo run --release -- run --terrain-metrics)
if [[ $# -gt 0 ]]; then
  cmd+=("$@")
fi

cd "${REPO_ROOT}"
echo "> ${cmd[*]}"
"${cmd[@]}"
