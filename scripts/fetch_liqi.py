#!/usr/bin/env python3
"""Fetch the latest liqi.json from the Mahjong Soul CDN.

Used by `.github/workflows/auto-liqi.yml`. Writes the JSON to
`src/bridge/majsoul/liqi.json` (relative to repo root) and reports
whether the contents changed via `$GITHUB_OUTPUT`. The proto
regeneration step is the workflow's responsibility (pbjs).

Outputs:
  version       Game version string (e.g. "0.10.305.w").
  liqi_prefix   Resource prefix for liqi.json (e.g. "v0.10.305.w/1").
  changed       "true" if liqi.json contents changed, else "false".
"""

from __future__ import annotations

import hashlib
import os
import sys
from pathlib import Path

import requests

REPO_ROOT = Path(__file__).resolve().parents[1]
LIQI_JSON_PATH = REPO_ROOT / "src" / "bridge" / "majsoul" / "liqi.json"

GAME_BASE = "https://game.maj-soul.com/1"


def fetch_version() -> str:
    resp = requests.get(f"{GAME_BASE}/version.json", timeout=15)
    resp.raise_for_status()
    return resp.json()["version"]


def fetch_liqi_prefix(version: str) -> str:
    resp = requests.get(f"{GAME_BASE}/resversion{version}.json", timeout=15)
    resp.raise_for_status()
    prefix = (
        resp.json().get("res", {}).get("res/proto/liqi.json", {}).get("prefix")
    )
    if not prefix:
        raise RuntimeError(
            f"resversion{version}.json missing res/proto/liqi.json prefix"
        )
    return prefix


def fetch_liqi_json(prefix: str) -> bytes:
    url = f"{GAME_BASE}/{prefix}/res/proto/liqi.json"
    resp = requests.get(url, timeout=30)
    resp.raise_for_status()
    return resp.content


def emit_output(name: str, value: str) -> None:
    out_path = os.environ.get("GITHUB_OUTPUT")
    if not out_path:
        print(f"::set-output (fallback) {name}={value}")
        return
    with open(out_path, "a", encoding="utf-8") as fh:
        fh.write(f"{name}={value}\n")


def main() -> int:
    version = fetch_version()
    prefix = fetch_liqi_prefix(version)
    print(f"[fetch_liqi] version={version} prefix={prefix}", flush=True)

    new_bytes = fetch_liqi_json(prefix)
    new_sha = hashlib.sha256(new_bytes).hexdigest()

    old_sha = ""
    if LIQI_JSON_PATH.exists():
        old_sha = hashlib.sha256(LIQI_JSON_PATH.read_bytes()).hexdigest()

    changed = old_sha != new_sha
    print(
        f"[fetch_liqi] old_sha={old_sha or '<missing>'} new_sha={new_sha} "
        f"changed={changed}",
        flush=True,
    )

    if changed:
        LIQI_JSON_PATH.parent.mkdir(parents=True, exist_ok=True)
        LIQI_JSON_PATH.write_bytes(new_bytes)
        print(f"[fetch_liqi] wrote {LIQI_JSON_PATH}", flush=True)

    emit_output("version", version)
    emit_output("liqi_prefix", prefix)
    emit_output("changed", "true" if changed else "false")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except requests.RequestException as exc:
        print(f"[fetch_liqi] HTTP error: {exc}", file=sys.stderr)
        sys.exit(2)
