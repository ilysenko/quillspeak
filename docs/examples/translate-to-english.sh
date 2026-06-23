#!/usr/bin/env bash
set -euo pipefail

text="${1:-}"
if [ -z "$text" ]; then
  exit 0
fi

backend="${QUILLSPEAK_TRANSLATOR_BACKEND:-claude}"

case "$backend" in
  claude)
    if ! command -v claude >/dev/null 2>&1; then
      printf '%s\n' "$text"
      exit 0
    fi

    prompt="Translate the following text to natural English. Preserve meaning, names, numbers, and formatting. Return only the translated text.

Text:
$text"

    claude -p "$prompt" \
      --bare \
      --safe-mode \
      --strict-mcp-config \
      --tools "" \
      --no-session-persistence \
      --model "${QUILLSPEAK_CLAUDE_MODEL:-haiku}" \
      --fallback-model "${QUILLSPEAK_CLAUDE_FALLBACK:-sonnet}" \
      --effort low
    ;;
  plain)
    printf '%s\n' "$text"
    ;;
  *)
    printf 'Unknown QUILLSPEAK_TRANSLATOR_BACKEND: %s\n' "$backend" >&2
    exit 2
    ;;
esac
