#!/usr/bin/env bash
# Install koma's git identity hooks into this clone.
# Sets core.hooksPath to scripts/githooks (tracked, shared).
#
# Usage:
#   ./scripts/install-git-hooks.sh
#   ./scripts/install-git-hooks.sh --check   # non-zero if hooks not installed

set -euo pipefail

ROOT=$(CDPATH= cd -P "$(dirname "$0")/.." && pwd)
cd "$ROOT"

HOOKS_PATH=scripts/githooks

[ -d "$HOOKS_PATH" ] || {
  echo "install-git-hooks: missing $HOOKS_PATH" >&2
  exit 1
}

chmod +x "$HOOKS_PATH"/* scripts/check-git-identity.sh 2>/dev/null || true

if [ "${1:-}" = "--check" ]; then
  current=$(git config --get core.hooksPath || true)
  if [ "$current" = "$HOOKS_PATH" ] || [ "$current" = "$ROOT/$HOOKS_PATH" ]; then
    echo "git hooks OK (core.hooksPath=$current)"
    exit 0
  fi
  echo "git hooks NOT installed (core.hooksPath=${current:-<unset>}; expected $HOOKS_PATH)" >&2
  exit 1
fi

git config core.hooksPath "$HOOKS_PATH"
echo "Installed: git config core.hooksPath=$HOOKS_PATH"
echo "Blocklist: scripts/git-identity-blocklist.txt"
echo "Checker:   scripts/check-git-identity.sh"
echo
echo "Hooks active for this clone:"
echo "  commit-msg  — blocks bot authors/trailers on every commit"
echo "  pre-push    — blocks pushing commits with bot identities"
echo "  update      — blocks ref updates that land bot identities"
echo
# Smoke-test the checker against a known-bad trailer.
if printf 'test\n\nCo-authored-by: Cursor <cursoragent@cursor.com>\n' \
  | scripts/check-git-identity.sh --stdin --author 'human <h@example.com>' --committer 'human <h@example.com>' \
  >/dev/null 2>&1
then
  echo "WARNING: smoke test did not reject Cursor trailer" >&2
  exit 1
fi
echo "Smoke test: Cursor trailer correctly rejected."
