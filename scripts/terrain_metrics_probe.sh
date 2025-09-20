#!/usr/bin/env bash
set -euo pipefail

# Run the terrain metrics probe (`--terrain-metrics`) in release mode.
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
