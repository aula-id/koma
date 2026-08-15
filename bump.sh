#!/bin/sh
# Update koma's release version declarations without committing or tagging.

set -eu

usage() {
    echo "Usage: $0 OLD NEW" >&2
    exit 2
}

fail() {
    echo "bump.sh: $*" >&2
    exit 1
}

[ "$#" -eq 2 ] || usage
OLD=$1
NEW=$2

validate_semver() {
    value=$1
    case "$value" in
        ''|*[!0-9.]*|.*|*.|*..*) return 1 ;;
    esac

    saved_ifs=$IFS
    IFS=.
    set -- $value
    IFS=$saved_ifs
    [ "$#" -eq 3 ] || return 1

    for component in "$@"; do
        case "$component" in
            0|[1-9]|[1-9][0-9]*) ;;
            *) return 1 ;;
        esac
    done
}

validate_semver "$OLD" || fail "OLD must be canonical stable SemVer (MAJOR.MINOR.PATCH): $OLD"
validate_semver "$NEW" || fail "NEW must be canonical stable SemVer (MAJOR.MINOR.PATCH): $NEW"
[ "$OLD" != "$NEW" ] || fail "OLD and NEW must differ"

ROOT=$(CDPATH= cd -P "$(dirname "$0")" && pwd)
cd "$ROOT"

FILES='src-agent/Cargo.toml flake.nix version.json Cargo.lock'
for file in $FILES; do
    [ -f "$file" ] || fail "required file is missing or not regular: $file"
    [ ! -L "$file" ] || fail "required file must not be a symlink: $file"
done

command -v awk >/dev/null 2>&1 || fail "awk is required"
command -v cargo >/dev/null 2>&1 || fail "cargo is required"

if command -v python3 >/dev/null 2>&1; then
    JSON_TOOL=python3
    if ! python3 - "$OLD" version.json <<'PY'
import json
import sys

expected, path = sys.argv[1:]
try:
    with open(path, encoding="utf-8") as stream:
        value = json.load(stream)
except (OSError, UnicodeError, json.JSONDecodeError) as error:
    print(f"bump.sh: invalid version.json: {error}", file=sys.stderr)
    raise SystemExit(1)
if not isinstance(value, dict) or type(value.get("version")) is not str or value["version"] != expected:
    print(f"bump.sh: version.json version is not exactly {expected}", file=sys.stderr)
    raise SystemExit(1)
PY
    then
        exit 1
    fi
elif command -v jq >/dev/null 2>&1; then
    JSON_TOOL=jq
    jq -e --arg old "$OLD" 'type == "object" and (.version | type == "string") and .version == $old' version.json >/dev/null \
        || fail "version.json is invalid or its version is not exactly $OLD"
else
    fail "version.json validation requires python3 or jq; neither was found"
fi

extract_cargo_toml() {
    awk '
        /^\[/ { in_package = ($0 == "[package]") }
        in_package && /^[[:space:]]*version[[:space:]]*=[[:space:]]*"[^"]*"[[:space:]]*$/ {
            value = $0
            sub(/^[^"]*"/, "", value)
            sub(/"[[:space:]]*$/, "", value)
            print value
            count++
        }
        END { if (count != 1) exit 1 }
    ' "$1"
}

extract_flake() {
    awk '
        /^[[:space:]]*version[[:space:]]*=[[:space:]]*"[^"]*"[[:space:]]*;[[:space:]]*$/ {
            value = $0
            sub(/^[^"]*"/, "", value)
            sub(/"[[:space:]]*;[[:space:]]*$/, "", value)
            print value
            count++
        }
        END { if (count != 1) exit 1 }
    ' "$1"
}

extract_json() {
    awk '
        /^[[:space:]]*"version"[[:space:]]*:[[:space:]]*"[^"]*"[[:space:]]*,?[[:space:]]*$/ {
            value = $0
            sub(/^[^:]*:[[:space:]]*"/, "", value)
            sub(/"[[:space:]]*,?[[:space:]]*$/, "", value)
            print value
            count++
        }
        END { if (count != 1) exit 1 }
    ' "$1"
}

extract_lock() {
    awk '
        /^\[\[package\]\][[:space:]]*$/ { in_agent = 0 }
        /^name[[:space:]]*=[[:space:]]*"agent"[[:space:]]*$/ { in_agent = 1; names++ }
        in_agent && /^version[[:space:]]*=[[:space:]]*"[^"]*"[[:space:]]*$/ {
            value = $0
            sub(/^[^"]*"/, "", value)
            sub(/"[[:space:]]*$/, "", value)
            print value
            versions++
            in_agent = 0
        }
        END { if (names != 1 || versions != 1) exit 1 }
    ' "$1"
}

check_fields() {
    expected=$1
    cargo_toml_v=$(extract_cargo_toml src-agent/Cargo.toml) \
        || fail "src-agent/Cargo.toml must have exactly one [package] version"
    flake_v=$(extract_flake flake.nix) \
        || fail "flake.nix must have exactly one version binding"
    json_v=$(extract_json version.json) \
        || fail "version.json must have exactly one version property line"
    lock_v=$(extract_lock Cargo.lock) \
        || fail "Cargo.lock must have exactly one agent package version"

    [ "$cargo_toml_v" = "$flake_v" ] &&
        [ "$cargo_toml_v" = "$json_v" ] &&
        [ "$cargo_toml_v" = "$lock_v" ] \
        || fail "release declarations are inconsistent (Cargo.toml=$cargo_toml_v, flake.nix=$flake_v, version.json=$json_v, Cargo.lock=$lock_v)"
    [ "$cargo_toml_v" = "$expected" ] \
        || fail "release declarations are $cargo_toml_v, not expected version $expected"
}

# Complete every preflight before creating or replacing any candidate.
check_fields "$OLD"

TMP_BASE=${TMPDIR:-/tmp}
TMP_DIR=$(mktemp -d "$TMP_BASE/koma-bump.XXXXXX") || fail "could not create temporary directory"
writes_started=0
committed=0

cleanup() {
    status=$?
    trap - 0 HUP INT TERM
    if [ "$writes_started" -eq 1 ] && [ "$committed" -eq 0 ]; then
        rollback_ok=1
        for file in $FILES; do
            if ! cat "$TMP_DIR/original/$file" > "$file"; then
                rollback_ok=0
            fi
        done
        if [ "$rollback_ok" -eq 1 ]; then
            echo "bump.sh: update failed; original files restored" >&2
        else
            echo "bump.sh: ERROR: update failed and rollback was incomplete; backups remain in $TMP_DIR/original" >&2
            exit 1
        fi
    fi
    rm -rf "$TMP_DIR"
    exit "$status"
}
trap cleanup 0
trap 'exit 1' HUP INT TERM

mkdir -p "$TMP_DIR/original/src-agent" "$TMP_DIR/candidate/src-agent" "$TMP_DIR/rendered/src-agent"
for file in $FILES; do
    cp -p "$file" "$TMP_DIR/original/$file"
    cp -p "$file" "$TMP_DIR/candidate/$file"
done

rewrite_quoted_field() {
    mode=$1
    input=$2
    output=$3
    awk -v mode="$mode" -v old="$OLD" -v new="$NEW" '
        function replace_value(line,    position) {
            position = index(line, old)
            if (position == 0) return line
            return substr(line, 1, position - 1) new substr(line, position + length(old))
        }
        /^\[/ && mode == "cargo" { in_target = ($0 == "[package]") }
        /^\[\[package\]\][[:space:]]*$/ && mode == "lock" { in_target = 0 }
        mode == "lock" && /^name[[:space:]]*=[[:space:]]*"agent"[[:space:]]*$/ { in_target = 1 }
        {
            target = 0
            if (mode == "cargo" && in_target && $0 ~ /^[[:space:]]*version[[:space:]]*=[[:space:]]*"[^"]*"[[:space:]]*$/) target = 1
            if (mode == "flake" && $0 ~ /^[[:space:]]*version[[:space:]]*=[[:space:]]*"[^"]*"[[:space:]]*;[[:space:]]*$/) target = 1
            if (mode == "json" && $0 ~ /^[[:space:]]*"version"[[:space:]]*:[[:space:]]*"[^"]*"[[:space:]]*,?[[:space:]]*$/) target = 1
            if (mode == "lock" && in_target && $0 ~ /^version[[:space:]]*=[[:space:]]*"[^"]*"[[:space:]]*$/) target = 1
            if (target) {
                if (index($0, old) == 0) exit 2
                $0 = replace_value($0)
                changed++
                if (mode == "lock") in_target = 0
            }
            print
        }
        END { if (changed != 1) exit 1 }
    ' "$input" > "$output"
}

rewrite_quoted_field cargo src-agent/Cargo.toml "$TMP_DIR/rendered/src-agent/Cargo.toml"
rewrite_quoted_field flake flake.nix "$TMP_DIR/rendered/flake.nix"
rewrite_quoted_field json version.json "$TMP_DIR/rendered/version.json"
rewrite_quoted_field lock Cargo.lock "$TMP_DIR/rendered/Cargo.lock"

for file in $FILES; do
    cat "$TMP_DIR/rendered/$file" > "$TMP_DIR/candidate/$file"
    cmp -s "$file" "$TMP_DIR/candidate/$file" && fail "candidate did not change $file"
done

# Re-run field and JSON checks against candidates before touching repository files.
(
    cd "$TMP_DIR/candidate"
    check_fields "$NEW"
    if [ "$JSON_TOOL" = python3 ]; then
        python3 - "$NEW" version.json <<'PY'
import json
import sys
expected, path = sys.argv[1:]
with open(path, encoding="utf-8") as stream:
    value = json.load(stream)
if not isinstance(value, dict) or type(value.get("version")) is not str or value["version"] != expected:
    raise SystemExit(1)
PY
    else
        jq -e --arg new "$NEW" 'type == "object" and (.version | type == "string") and .version == $new' version.json >/dev/null
    fi
) || fail "candidate validation failed"

writes_started=1
for file in $FILES; do
    cat "$TMP_DIR/candidate/$file" > "$file"
done

if ! cargo metadata --locked --no-deps --format-version 1 >/dev/null; then
    fail "cargo metadata validation failed"
fi
check_fields "$NEW"

if [ "$JSON_TOOL" = python3 ]; then
    python3 -m json.tool version.json >/dev/null || fail "post-update version.json validation failed"
else
    jq -e . version.json >/dev/null || fail "post-update version.json validation failed"
fi

committed=1
printf 'Version bumped: %s -> %s\nChanged files:\n' "$OLD" "$NEW"
for file in $FILES; do
    printf '  %s\n' "$file"
done
printf 'Commit and tag when ready.\n'
