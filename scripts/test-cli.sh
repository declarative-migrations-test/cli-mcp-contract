#!/usr/bin/env bash
set -euo pipefail
DPM="${DPM_BIN:?DPM_BIN is required}"
ADMIN="${POSTGRES_ADMIN_URL:-postgres://postgres@localhost:5432/postgres}"
DB="dm_cli_mcp_contract"
TARGET="postgres://postgres@localhost:5432/${DB}"
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cleanup() { psql "$ADMIN" -v ON_ERROR_STOP=1 -c "DROP DATABASE IF EXISTS ${DB} WITH (FORCE)" >/dev/null 2>&1 || true; }
trap cleanup EXIT
cleanup
psql "$ADMIN" -v ON_ERROR_STOP=1 -c "CREATE DATABASE ${DB}" >/dev/null
"$DPM" help | grep -q -- '--allow-destructive-ops'
"$DPM" apply --source-sql "$root/fixtures/v1.sql" --target "$TARGET" --shadow "$ADMIN" --yes
set +e
"$DPM" diff --source-sql "$root/fixtures/v2.sql" --target "$TARGET" --shadow "$ADMIN" --fail-on-diff >/dev/null
status=$?
set -e
test "$status" -eq 2
SOURCE_SQL_FILE="$root/fixtures/v2.sql" TARGET_DATABASE_URL="$TARGET" SHADOW_DATABASE_URL="$ADMIN" DPM_FORMAT=json "$DPM" diff > "$root/artifacts/plan.json"
python3 -m json.tool "$root/artifacts/plan.json" >/dev/null
DPM_REAL_BIN="$DPM" DPM_REAL_TARGET="$TARGET" DPM_REAL_SHADOW="$ADMIN" python3 -m unittest -v tests/test_mcp.py

echo "CLI and MCP contract certification passed"
