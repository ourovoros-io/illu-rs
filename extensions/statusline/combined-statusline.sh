#!/bin/bash
# Combined statusline: model + repo + context + illu status
#
# Format: ▸ model · folder › branch  ▰▰▱▱ ctx%  ◆ illu: status
#
# Install:
#   cp extensions/statusline/combined-statusline.sh ~/.claude/statusline.sh
#   chmod +x ~/.claude/statusline.sh
#
#   In ~/.claude/settings.json:
#   {
#     "statusLine": {
#       "command": "~/.claude/statusline.sh"
#     }
#   }

stdin_data=$(cat)

# ── Parse Claude Code JSON input ──

IFS=$'\t' read -r current_dir model_name duration_ms ctx_used cache_pct < <(
    echo "$stdin_data" | jq -r '[
        .workspace.current_dir // "unknown",
        .model.display_name // "Unknown",
        (.cost.total_duration_ms // 0),
        (try (
            if (.context_window.remaining_percentage // null) != null then
                100 - (.context_window.remaining_percentage | floor)
            elif (.context_window.context_window_size // 0) > 0 then
                (((.context_window.current_usage.input_tokens // 0) +
                  (.context_window.current_usage.cache_creation_input_tokens // 0) +
                  (.context_window.current_usage.cache_read_input_tokens // 0)) * 100 /
                 .context_window.context_window_size) | floor
            else "null" end
        ) catch "null"),
        (try (
            (.context_window.current_usage // {}) |
            if (.input_tokens // 0) + (.cache_read_input_tokens // 0) > 0 then
                ((.cache_read_input_tokens // 0) * 100 /
                 ((.input_tokens // 0) + (.cache_read_input_tokens // 0))) | floor
            else 0 end
        ) catch 0)
    ] | @tsv'
)

: "${current_dir:=unknown}"
: "${model_name:=Unknown}"
: "${duration_ms:=0}"

# ── Git info ──

git_branch=""
git_root=""
if cd "$current_dir" 2>/dev/null; then
    git_branch=$(git -c core.useBuiltinFSMonitor=false branch --show-current 2>/dev/null)
    git_root=$(git -c core.useBuiltinFSMonitor=false rev-parse --show-toplevel 2>/dev/null)
fi

if [ -n "$git_root" ]; then
    folder_name=$(basename "$git_root")
else
    folder_name=$(basename "$current_dir")
fi

# ── illu status ──

illu_status=""
status_file="${git_root:-.}/.illu/status"
if [ -f "$status_file" ]; then
    illu_status=$(cat "$status_file" 2>/dev/null)
fi

# ── Colors ──

C='\033[36m'       # Cyan
G='\033[32m'       # Green
Y='\033[33m'       # Yellow/Amber
RED='\033[31m'     # Red
W='\033[97m'       # Bright white
D='\033[2m'        # Dim
R='\033[0m'        # Reset

# ── Context progress bar ──

progress_bar=""
ctx_pct=""
bar_width=10

if [ -n "$ctx_used" ] && [ "$ctx_used" != "null" ]; then
    filled=$((ctx_used * bar_width / 100))
    empty=$((bar_width - filled))

    if [ "$ctx_used" -lt 50 ]; then
        bar_color="$C"
    elif [ "$ctx_used" -lt 80 ]; then
        bar_color="$Y"
    else
        bar_color="$RED"
    fi

    progress_bar="${bar_color}"
    for ((i=0; i<filled; i++)); do progress_bar="${progress_bar}▰"; done
    progress_bar="${progress_bar}${D}"
    for ((i=0; i<empty; i++)); do progress_bar="${progress_bar}▱"; done
    progress_bar="${progress_bar}${R}"

    ctx_pct="${bar_color}${ctx_used}%${R}"
fi

# ── Session duration ──

session_time=""
if [ "$duration_ms" -gt 0 ] 2>/dev/null; then
    total_sec=$((duration_ms / 1000))
    hours=$((total_sec / 3600))
    minutes=$(((total_sec % 3600) / 60))
    seconds=$((total_sec % 60))
    if [ "$hours" -gt 0 ]; then
        session_time="${hours}h${minutes}m"
    elif [ "$minutes" -gt 0 ]; then
        session_time="${minutes}m${seconds}s"
    else
        session_time="${seconds}s"
    fi
fi

# ── Model name ──

short_model=$(echo "$model_name" | sed -E 's/Claude [0-9.]+ //; s/^Claude //' | tr '[:upper:]' '[:lower:]')

# ── Assemble ──

SEP="${D} · ${R}"

line=$(printf "${C}▸${R} ${W}%s${R}" "$short_model")
line="${line}$(printf '%b%s' "$SEP" "$folder_name")"

if [ -n "$git_branch" ]; then
    line="${line}$(printf ' %b›%b %b%s%b' "$D" "$R" "$C" "$git_branch" "$R")"
fi

if [ -n "$progress_bar" ]; then
    line="${line}$(printf '  %b %b' "$progress_bar" "$ctx_pct")"
fi

if [ -n "$session_time" ]; then
    line="${line}$(printf '%b%b%s%b' "$SEP" "$D" "$session_time" "$R")"
fi

if [ "$cache_pct" -gt 0 ] 2>/dev/null; then
    line="${line}$(printf ' %b↻%s%%%b' "$D" "$cache_pct" "$R")"
fi

# ── illu status (appended at the end) ──

if [ -n "$illu_status" ]; then
    case "$illu_status" in
        ready)
            line="${line}$(printf '  %b◆%b%b illu%b' "$G" "$R" "$D" "$R")"
            ;;
        indexing*|refreshing*)
            line="${line}$(printf '  %b◆ illu: %s%b' "$Y" "$illu_status" "$R")"
            ;;
        fetching*)
            line="${line}$(printf '  %b◆ illu: %s%b' "$C" "$illu_status" "$R")"
            ;;
        *)
            line="${line}$(printf '  %b◆%b illu: %s' "$C" "$R" "$illu_status")"
            ;;
    esac
fi

printf '%b' "$line"
