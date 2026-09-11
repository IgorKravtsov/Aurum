#!/usr/bin/env sh

set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
SOURCE_SKILL="$SCRIPT_DIR/skills/aurum/SKILL.md"
AGENTS_DESTINATION="${AGENTS_SKILLS_DIR:-${HOME}/.agents/skills}/aurum"
CLAUDE_DESTINATION="${CLAUDE_SKILLS_DIR:-${HOME}/.claude/skills}/aurum"

if [ ! -f "$SOURCE_SKILL" ]; then
  echo "Aurum skill source not found: $SOURCE_SKILL" >&2
  exit 1
fi

install -d -m 0755 "$AGENTS_DESTINATION" "$CLAUDE_DESTINATION"
install -m 0644 "$SOURCE_SKILL" "$AGENTS_DESTINATION/SKILL.md"
install -m 0644 "$SOURCE_SKILL" "$CLAUDE_DESTINATION/SKILL.md"

echo "Aurum skill installed to:"
echo "  $AGENTS_DESTINATION"
echo "  $CLAUDE_DESTINATION"
