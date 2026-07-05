#!/usr/bin/env sh
set -eu

URL="${1:-https://raw.githubusercontent.com/dysolix/hasagi-types/main/swagger.json}"
OUT_FILE="${2:-schema/swagger.json}"
TMP_FILE="${OUT_FILE}.tmp"

mkdir -p "$(dirname "$OUT_FILE")"

if command -v curl >/dev/null 2>&1; then
    curl -fsSL "$URL" -o "$TMP_FILE"
elif command -v wget >/dev/null 2>&1; then
    wget -qO "$TMP_FILE" "$URL"
else
    echo "curl or wget is required" >&2
    exit 1
fi

python - "$TMP_FILE" <<'PY'
import json
import sys

path = sys.argv[1]
with open(path, "r", encoding="utf-8") as handle:
    document = json.load(handle)

if "openapi" not in document or "paths" not in document:
    raise SystemExit("Downloaded file is not an OpenAPI document with paths.")

schema_count = len(document.get("components", {}).get("schemas", {}))
print(f"OpenAPI: {document.get('openapi')}")
print(f"Version: {document.get('info', {}).get('version')}")
print(f"Paths: {len(document.get('paths', {}))}")
print(f"Schemas: {schema_count}")
PY

mv "$TMP_FILE" "$OUT_FILE"
echo "Updated $OUT_FILE"
