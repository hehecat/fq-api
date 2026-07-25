#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"
export FQ_KEY_FILE="${FQ_KEY_FILE:-$PWD/data/crypt_keys.json}"
export FQ_FONT_MAP_DIR="${FQ_FONT_MAP_DIR:-$PWD/font_maps}"
export FQ_CHAPTER_CACHE="${FQ_CHAPTER_CACHE:-$PWD/data/chapters/decoded_json}"
export FQ_PLAINTEXT_DIR="${FQ_PLAINTEXT_DIR:-$PWD/data/chapters/plaintext}"
export FQ_ENABLE_ADB="${FQ_ENABLE_ADB:-false}"
export PORT="${PORT:-18080}"
exec ./target/release/fq-api
