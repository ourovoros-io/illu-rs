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

# Build a ▰▱ progress bar from "current/total" or "[current/total]"
illu_progress_bar() {
    local status="$1" color="$2" width=8
    local current total

    if [[ "$status" =~ \[([0-9]+)/([0-9]+)\] ]]; then
        current="${BASH_REMATCH[1]}"
        total="${BASH_REMATCH[2]}"
    elif [[ "$status" =~ ([0-9]+)/([0-9]+) ]]; then
        current="${BASH_REMATCH[1]}"
        total="${BASH_REMATCH[2]}"
    else
        return 1
    fi

    [ "$total" -eq 0 ] && return 1
    local filled=$((current * width / total))
    local empty=$((width - filled))
    local bar="${color}"
    for ((j=0; j<filled; j++)); do bar="${bar}▰"; done
    bar="${bar}${DIM}"
    for ((j=0; j<empty; j++)); do bar="${bar}▱"; done
    bar="${bar}${RESET}"
    printf '%b' "$bar"
    return 0
}

# Format illu status
if [ -n "$illu_status" ]; then
    case "$illu_status" in
        ready)
            printf "${GREEN}◆${RESET}${DIM} illu${RESET}"
            ;;
        indexing*|refreshing*)
            printf "${YELLOW}◆${RESET} ${YELLOW}illu: %s${RESET}" "$illu_status"
            if illu_bar=$(illu_progress_bar "$illu_status" "$YELLOW"); then
                printf " %b" "$illu_bar"
            fi
            ;;
        fetching*)
            printf "${CYAN}◆${RESET} ${CYAN}illu: %s${RESET}" "$illu_status"
            if illu_bar=$(illu_progress_bar "$illu_status" "$CYAN"); then
                printf " %b" "$illu_bar"
            fi
            ;;
        *)
            printf "${CYAN}◆${RESET} illu: %s" "$illu_status"
            ;;
    esac
fi
