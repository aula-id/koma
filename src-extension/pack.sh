#!/bin/bash
set -euo pipefail

# Resolve script directory regardless of CWD
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

# Create dist directory if missing
mkdir -p dist

# The examples to package
EXAMPLES=(
  "echo-tool-daemon"
  "upper-tool-oneshot"
  "fleet-board-daemon"
  "agent-peek-oneshot"
  "oauth-demo-daemon"
  "event-watcher-daemon"
  "orchestrator-daemon"
  "tui-demo-daemon"
)

echo "=== Building extensions ==="
cargo build --workspace --release

echo ""
echo "=== Packaging extensions ==="

# Determine which tool to use for JSON editing
if command -v jq &> /dev/null; then
  JSON_TOOL="jq"
else
  JSON_TOOL="python3"
fi

for example in "${EXAMPLES[@]}"; do
  example_dir="example/$example"
  manifest_src="$example_dir/manifest.json"
  binary_src="target/release/$example"

  # Create temp staging directory
  stage_dir=$(mktemp -d)
  trap "rm -rf '$stage_dir'" EXIT

  # Create bin directory in staging
  mkdir -p "$stage_dir/bin"

  # Copy and modify manifest.json
  if [ "$JSON_TOOL" = "jq" ]; then
    jq ".runtime.exec = \"bin/$example\"" "$manifest_src" > "$stage_dir/manifest.json"
  else
    python3 -c "
import json
import sys
with open('$manifest_src', 'r') as f:
  data = json.load(f)
data['runtime']['exec'] = 'bin/$example'
with open('$stage_dir/manifest.json', 'w') as f:
  json.dump(data, f, indent=2)
"
  fi

  # Copy release binary
  cp "$binary_src" "$stage_dir/bin/$example"

  # Copy the extension's own UI (contributes.panels), if it has one — the zip
  # layout becomes manifest.json + bin/<name> + ui/... . koma serves this dir
  # straight off disk at `koma://extension/<id>/...` once installed.
  zip_paths=(manifest.json bin/)
  if [ -d "$example_dir/ui" ]; then
    cp -r "$example_dir/ui" "$stage_dir/ui"
    zip_paths+=(ui/)
  fi

  # Create zip from the staging directory contents
  # Navigate into the staging directory so zip root contains manifest.json,
  # bin/, and (if present) ui/
  (cd "$stage_dir" && zip -q -r "$SCRIPT_DIR/dist/$example.zip" "${zip_paths[@]}")

  # Clean up temp directory
  rm -rf "$stage_dir"

  echo "Packaged: dist/$example.zip"
done

echo ""
echo "=== Summary ==="
echo "Distributable packages created:"
for example in "${EXAMPLES[@]}"; do
  zip_path="dist/$example.zip"
  if [ -f "$zip_path" ]; then
    size=$(du -h "$zip_path" | cut -f1)
    echo "  $zip_path ($size)"
  fi
done
