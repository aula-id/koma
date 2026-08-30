#!/usr/bin/env bash
# Shared checker: reject commits whose author/committer/trailers match the
# identity blocklist. Used by local hooks and CI.
#
# Usage:
#   check-git-identity.sh --message-file FILE [--author "N <e>"] [--committer "N <e>"]
#   check-git-identity.sh --range A..B
#   check-git-identity.sh --commit SHA
#   check-git-identity.sh --stdin   # reads a full commit message from stdin
#
# Exit 0 = clean, 1 = blocked identity found, 2 = usage/setup error.

set -euo pipefail

SCRIPT_DIR=$(CDPATH= cd -P "$(dirname "$0")" && pwd)
ROOT=$(CDPATH= cd -P "$SCRIPT_DIR/.." && pwd)
BLOCKLIST=${KOMA_IDENTITY_BLOCKLIST:-"$SCRIPT_DIR/git-identity-blocklist.txt"}

die() { echo "check-git-identity: $*" >&2; exit 2; }
fail() { echo "check-git-identity: BLOCKED: $*" >&2; exit 1; }

[ -f "$BLOCKLIST" ] || die "blocklist not found: $BLOCKLIST"

# Load patterns (lowercase) into a bash array.
mapfile -t PATTERNS < <(
  awk '
    /^[[:space:]]*#/ { next }
    /^[[:space:]]*$/ { next }
    {
      line = $0
      sub(/^[[:space:]]+/, "", line)
      sub(/[[:space:]]+$/, "", line)
      if (line != "") print tolower(line)
    }
  ' "$BLOCKLIST"
)
[ "${#PATTERNS[@]}" -gt 0 ] || die "blocklist is empty: $BLOCKLIST"

identity_hits() {
  # $1 = label, $2 = free-form identity text
  local label=$1
  local text
  text=$(printf '%s' "${2:-}" | tr '[:upper:]' '[:lower:]')
  [ -n "$text" ] || return 0
  local p
  for p in "${PATTERNS[@]}"; do
    case "$text" in
      *"$p"*) echo "$label matches blocklist pattern '$p' ← $2" ;;
    esac
  done
}

check_message_trailers() {
  # Scan full commit message for Co-authored-by / Signed-off-by / Reviewed-by.
  local msg=$1
  local line name_email hits
  while IFS= read -r line || [ -n "$line" ]; do
    case "$line" in
      [Cc][Oo]-[Aa][Uu][Tt][Hh][Oo][Rr][Ee][Dd]-[Bb][Yy]:*|\
      [Ss][Ii][Gg][Nn][Ee][Dd]-[Oo][Ff][Ff]-[Bb][Yy]:*|\
      [Rr][Ee][Vv][Ii][Ee][Ww][Ee][Dd]-[Bb][Yy]:*|\
      [Aa][Cc][Kk][Nn][Oo][Ww][Ll][Ee][Dd][Gg][Ee][Dd]-[Bb][Yy]:*)
        name_email=${line#*:}
        name_email=${name_email## }
        hits=$(identity_hits "trailer '$line'" "$name_email" || true)
        if [ -n "${hits:-}" ]; then
          printf '%s\n' "$hits"
        fi
        ;;
    esac
  done <<EOF
$msg
EOF
}

check_one_commit() {
  local sha=$1
  local author committer msg hits
  author=$(git -C "$ROOT" log -1 --format='%an <%ae>' "$sha")
  committer=$(git -C "$ROOT" log -1 --format='%cn <%ce>' "$sha")
  msg=$(git -C "$ROOT" log -1 --format='%B' "$sha")
  hits=$(
    {
      identity_hits "author" "$author"
      identity_hits "committer" "$committer"
      check_message_trailers "$msg"
    } | sed '/^$/d'
  )
  if [ -n "${hits:-}" ]; then
    echo "commit $sha:"
    printf '%s\n' "$hits" | sed 's/^/  /'
    return 1
  fi
  return 0
}

MODE=""
MSG_FILE=""
AUTHOR=""
COMMITTER=""
RANGE=""
COMMIT=""

while [ "$#" -gt 0 ]; do
  case "$1" in
    --message-file) MSG_FILE=$2; MODE=message; shift 2 ;;
    --author) AUTHOR=$2; shift 2 ;;
    --committer) COMMITTER=$2; shift 2 ;;
    --range) RANGE=$2; MODE=range; shift 2 ;;
    --commit) COMMIT=$2; MODE=commit; shift 2 ;;
    --stdin) MODE=stdin; shift ;;
    -h|--help)
      sed -n '1,20p' "$0"
      exit 0
      ;;
    *) die "unknown arg: $1" ;;
  esac
done

[ -n "$MODE" ] || die "specify --message-file, --range, --commit, or --stdin"

case "$MODE" in
  message|stdin)
    if [ "$MODE" = message ]; then
      [ -n "$MSG_FILE" ] && [ -f "$MSG_FILE" ] || die "--message-file missing"
      MSG=$(cat "$MSG_FILE")
    else
      MSG=$(cat)
    fi
    # Resolve author/committer: explicit flag > git env > git config.
    if [ -z "$AUTHOR" ] || [ -z "${AUTHOR// }" ] || [ "$AUTHOR" = " <>" ]; then
      an=${GIT_AUTHOR_NAME:-$(git -C "$ROOT" config user.name 2>/dev/null || true)}
      ae=${GIT_AUTHOR_EMAIL:-$(git -C "$ROOT" config user.email 2>/dev/null || true)}
      AUTHOR="$an <$ae>"
    fi
    if [ -z "$COMMITTER" ] || [ -z "${COMMITTER// }" ] || [ "$COMMITTER" = " <>" ]; then
      cn=${GIT_COMMITTER_NAME:-$(git -C "$ROOT" config user.name 2>/dev/null || true)}
      ce=${GIT_COMMITTER_EMAIL:-$(git -C "$ROOT" config user.email 2>/dev/null || true)}
      COMMITTER="$cn <$ce>"
    fi
    HITS=$(
      {
        identity_hits "author" "$AUTHOR"
        identity_hits "committer" "$COMMITTER"
        check_message_trailers "$MSG"
      } | sed '/^$/d'
    )
    if [ -n "${HITS:-}" ]; then
      echo "check-git-identity: blocked identity in pending commit:" >&2
      printf '%s\n' "$HITS" | sed 's/^/  /' >&2
      echo "Remove the bot trailer/author or re-commit as a human." >&2
      echo "Blocklist: $BLOCKLIST" >&2
      exit 1
    fi
    ;;
  commit)
    [ -n "$COMMIT" ] || die "--commit needs a SHA"
    if ! check_one_commit "$COMMIT"; then
      fail "forbidden identity in $COMMIT"
    fi
    ;;
  range)
    [ -n "$RANGE" ] || die "--range needs A..B"
    # Empty range (e.g. PR with no commits) is fine.
    SHAS=$(git -C "$ROOT" rev-list "$RANGE" 2>/dev/null || true)
    BAD=0
    for sha in $SHAS; do
      if ! check_one_commit "$sha"; then
        BAD=1
      fi
    done
    if [ "$BAD" -ne 0 ]; then
      fail "forbidden identity in range $RANGE"
    fi
    ;;
esac

exit 0
