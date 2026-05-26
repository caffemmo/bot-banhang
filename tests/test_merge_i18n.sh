#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WORK_DIR="$(mktemp -d)"
trap 'rm -rf "$WORK_DIR"' EXIT

SOURCE_DIR="$WORK_DIR/source"
TARGET_DIR="$WORK_DIR/target"
mkdir -p "$SOURCE_DIR" "$TARGET_DIR"

cat >"$SOURCE_DIR/languages.json" <<'JSON'
[
  {"code":"vi","label":"Vietnamese default","fallback":"en","enabled":true},
  {"code":"en","label":"English","fallback":"en","enabled":true},
  {"code":"th","label":"Thai","fallback":"en","enabled":true}
]
JSON

cat >"$TARGET_DIR/languages.json" <<'JSON'
[
  {"code":"vi","label":"Tieng Viet custom","fallback":"en","enabled":true}
]
JSON

cat >"$SOURCE_DIR/vi.json" <<'JSON'
{
  "start": "Default start",
  "wallet": "Wallet"
}
JSON

cat >"$TARGET_DIR/vi.json" <<'JSON'
{
  "start": "Runtime custom start"
}
JSON

cat >"$SOURCE_DIR/en.json" <<'JSON'
{
  "start": "Start",
  "wallet": "Wallet"
}
JSON

cat >"$SOURCE_DIR/th.json" <<'JSON'
{
  "start": "Start TH"
}
JSON

bash "$ROOT_DIR/scripts/merge_i18n.sh" "$SOURCE_DIR" "$TARGET_DIR"

python3 - "$TARGET_DIR" <<'PY'
import json
import pathlib
import sys

target = pathlib.Path(sys.argv[1])

vi = json.loads((target / "vi.json").read_text(encoding="utf-8"))
assert vi["start"] == "Runtime custom start", vi
assert vi["wallet"] == "Wallet", vi

en = json.loads((target / "en.json").read_text(encoding="utf-8"))
assert en["wallet"] == "Wallet", en

th = json.loads((target / "th.json").read_text(encoding="utf-8"))
assert th["start"] == "Start TH", th

languages = json.loads((target / "languages.json").read_text(encoding="utf-8"))
by_code = {item["code"]: item for item in languages}
assert by_code["vi"]["label"] == "Tieng Viet custom", languages
assert by_code["en"]["label"] == "English", languages
assert by_code["th"]["label"] == "Thai", languages
PY
