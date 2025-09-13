#!/usr/bin/env bash
set -euo pipefail

# Flamegraph helper for geist-mesh-cpu benches.
# Requires cargo-flamegraph installed and perf (Linux) or dtrace (macOS) permissions.

CRATE="geist-mesh-cpu"
BENCH="wcc"
PROFILE="flamegraph"

FILTER="${1:-}"

echo "Using profile: $PROFILE (debug symbols, no LTO, 1 codegen-unit)"
echo "RUSTFLAGS: forcing frame pointers for better stacks"

export RUSTFLAGS="-C force-frame-pointers=yes ${RUSTFLAGS:-}"

cmd=(cargo flamegraph --root -p "$CRATE" --bench "$BENCH" --profile "$PROFILE")
if [[ -n "$FILTER" ]]; then
  # Pass through criterion filter after `--` to select a single bench
  cmd+=(-- "${FILTER}")
fi

echo "> ${cmd[*]}"
"${cmd[@]}"

echo
echo "Searching for generated flamegraphs..."
# Collect most recent flamegraphs (cargo-flamegraph writes to target/flamegraph.svg)
FLAMES=$(ls -t target/**/flamegraph*.svg 2>/dev/null | head -n 5 || true)

if [[ -z "$FLAMES" ]]; then
  echo "No flamegraph SVGs found under target/." >&2
  echo "Expected at: target/flamegraph.svg"
  exit 0
fi

echo "Found SVG(s):"
echo "$FLAMES" | while IFS= read -r f; do
  [[ -z "$f" ]] && continue
  echo "  $f"
done

# Open the SVGs depending on OS
uname_s=$(uname -s 2>/dev/null || echo unknown)
case "$uname_s" in
  Darwin)
    echo "$FLAMES" | while IFS= read -r f; do [[ -n "$f" ]] && open "$f" >/dev/null 2>&1 || true; done ;;
  Linux)
    if command -v xdg-open >/dev/null 2>&1; then
      echo "$FLAMES" | while IFS= read -r f; do [[ -n "$f" ]] && xdg-open "$f" >/dev/null 2>&1 || true; done
    else
      echo "xdg-open not found; please open the SVG(s) manually."
    fi
    ;;
  *)
    echo "Unrecognized OS ($uname_s). Please open the SVG(s) manually."
    ;;
esac

echo "Done."
