#!/usr/bin/env bash
set -euo pipefail

if [ "$#" -ne 2 ]; then
  echo "Usage: $0 SOURCE_I18N_DIR TARGET_I18N_DIR" >&2
  exit 2
fi

SOURCE_DIR="$1"
TARGET_DIR="$2"

if [ ! -d "$SOURCE_DIR" ]; then
  exit 0
fi

mkdir -p "$TARGET_DIR"

if [ -n "${MERGE_I18N_BIN:-}" ] && [ -x "$MERGE_I18N_BIN" ]; then
  "$MERGE_I18N_BIN" merge-i18n "$SOURCE_DIR" "$TARGET_DIR"
  exit 0
fi

for candidate in ./botbanhang ../botbanhang /opt/botbanhang/botbanhang; do
  if [ -x "$candidate" ]; then
    "$candidate" merge-i18n "$SOURCE_DIR" "$TARGET_DIR"
    exit 0
  fi
done

if ! command -v python3 >/dev/null 2>&1; then
  echo "Cannot merge i18n: set MERGE_I18N_BIN to a botbanhang binary or install python3." >&2
  exit 1
fi

python3 - "$SOURCE_DIR" "$TARGET_DIR" <<'PY'
import json
import pathlib
import shutil
import sys

source_dir = pathlib.Path(sys.argv[1])
target_dir = pathlib.Path(sys.argv[2])


def read_json(path):
    with path.open("r", encoding="utf-8") as handle:
        return json.load(handle)


def write_json(path, value):
    tmp_path = path.with_name(f".{path.name}.tmp")
    with tmp_path.open("w", encoding="utf-8") as handle:
        json.dump(value, handle, ensure_ascii=False, indent=2)
        handle.write("\n")
    tmp_path.replace(path)


def merge_language_registry(source, target):
    merged = []
    seen = set()
    for item in target:
        code = item.get("code") if isinstance(item, dict) else None
        if code and code not in seen:
            merged.append(item)
            seen.add(code)
    for item in source:
        code = item.get("code") if isinstance(item, dict) else None
        if code and code not in seen:
            merged.append(item)
            seen.add(code)
    return merged


def merge_json_file(source_path, target_path):
    if not target_path.exists():
        shutil.copy2(source_path, target_path)
        return

    source = read_json(source_path)
    target = read_json(target_path)

    if source_path.name == "languages.json" and isinstance(source, list) and isinstance(target, list):
        write_json(target_path, merge_language_registry(source, target))
    elif isinstance(source, dict) and isinstance(target, dict):
        merged = dict(source)
        merged.update(target)
        write_json(target_path, dict(sorted(merged.items())))


for source_path in sorted(source_dir.iterdir()):
    target_path = target_dir / source_path.name
    if source_path.is_dir():
        if not target_path.exists():
            shutil.copytree(source_path, target_path)
        continue

    if not source_path.is_file():
        continue

    if source_path.suffix.lower() == ".json":
        merge_json_file(source_path, target_path)
    elif not target_path.exists():
        shutil.copy2(source_path, target_path)
PY
