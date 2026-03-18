#!/bin/bash
# illu-rs statusline extension for Claude Code / Gemini CLI
#
# Shows illu indexing status alongside your existing statusline.
# Reads .illu/status from the current git repo root.
#
# Format: ◆ illu: <status>
#
# Usage:
#   1. Copy this to ~/.claude/illu-statusline.sh
#   2. Add to your Claude Code settings.json:
#      {
#        "statusLine": {
#          "command": "~/.claude/illu-statusline.sh"
#        }
#      }
#
#   Or combine with an existing statusline script (see below).

stdin_data=$(cat)

# Find the repo root from Claude's workspace data
current_dir=$(echo "$stdin_data" | jq -r '.workspace.current_dir // empty' 2>/dev/null)
if [ -z "$current_dir" ]; then
    current_dir=$(echo "$stdin_data" | jq -r '.cwd // empty' 2>/dev/null)
fi

# Try to find git root
git_root=""
if [ -n "$current_dir" ] && cd "$current_dir" 2>/dev/null; then
    git_root=$(git -c core.useBuiltinFSMonitor=false rev-parse --show-toplevel 2>/dev/null)
fi

# Read illu status
illu_status=""
status_file="${git_root:-.}/.illu/status"
if [ -f "$status_file" ]; then
    illu_status=$(cat "$status_file" 2>/dev/null)
fi

# Colors
CYAN='\033[36m'
DIM='\033[2m'
GREEN='\033[32m'
YELLOW='\033[33m'
RESET='\033[0m'

# Format illu status
if [ -n "$illu_status" ]; then
    case "$illu_status" in
        ready)
            printf "${GREEN}◆${RESET}${DIM} illu${RESET}"
            ;;
        indexing*|refreshing*)
            printf "${YELLOW}◆${RESET} ${YELLOW}illu: %s${RESET}" "$illu_status"
            ;;
        fetching*)
            printf "${CYAN}◆${RESET} ${CYAN}illu: %s${RESET}" "$illu_status"
            ;;
        *)
            printf "${CYAN}◆${RESET} illu: %s" "$illu_status"
            ;;
    esac
fi
