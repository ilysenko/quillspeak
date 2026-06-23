#!/usr/bin/env bash
set -euo pipefail

text="${1:-}"
if [ -z "${text//[[:space:]]/}" ]; then
  exit 0
fi

prompt_text() {
  printf '%s\n' 'Translate the text below to natural English.'
  printf '%s\n' 'Preserve meaning, names, numbers, and formatting.'
  printf '%s\n' 'If the text is already English, make it polite and natural.'
  printf '%s\n' 'Return only the final English text. No quotes, no markdown, no explanations.'
  printf '\nText:\n%s\n' "$text"
}

run_claude() {
  local model="${QUILLSPEAK_CLAUDE_MODEL:-haiku}"
  local fallback="${QUILLSPEAK_CLAUDE_FALLBACK:-sonnet}"
  local effort="${QUILLSPEAK_CLAUDE_EFFORT:-low}"

  if ! command -v claude >/dev/null 2>&1; then
    printf '%s\n' "$text"
    return 0
  fi

  claude -p "$(prompt_text)" \
    --bare \
    --safe-mode \
    --strict-mcp-config \
    --tools "" \
    --disallowedTools "mcp__*" \
    --no-session-persistence \
    --output-format text \
    --model "$model" \
    --fallback-model "$fallback" \
    --effort "$effort" \
    --system-prompt "You are a fast translation engine. Return only the translated or polished English text."
}

backend="${QUILLSPEAK_TRANSLATOR_BACKEND:-${TRANSLATE_BACKEND:-claude}}"

case "${backend,,}" in
  claude|cloud)
    run_claude
    ;;
  haiku|claude-haiku|cloud-haiku)
    QUILLSPEAK_CLAUDE_MODEL="${QUILLSPEAK_CLAUDE_MODEL:-haiku}" run_claude
    ;;
  sonnet|claude-sonnet|cloud-sonnet)
    QUILLSPEAK_CLAUDE_MODEL="${QUILLSPEAK_CLAUDE_MODEL:-sonnet}" run_claude
    ;;
  plain|echo|local)
    printf '%s\n' "$text"
    ;;
  *)
    printf 'Unknown QUILLSPEAK_TRANSLATOR_BACKEND=%q. Use claude, haiku, sonnet, or plain.\n' "$backend" >&2
    exit 2
    ;;
esac
