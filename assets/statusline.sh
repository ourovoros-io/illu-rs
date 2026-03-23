#!/bin/bash
# Futuristic minimal statusline
#
# Format: ▸ model · folder › branch  ▰▰▱▱ ctx% · duration  ↻cache%
#
# Context % uses Claude Code's pre-calculated remaining_percentage,
# which accounts for compaction reserves. 100% = compaction fires.

stdin_data=$(cat)

IFS=$'\t' read -r current_dir model_name lines_added lines_removed duration_ms ctx_used cache_pct < <(
    echo "$stdin_data" | jq -r '[
        .workspace.current_dir // "unknown",
        .model.display_name // "Unknown",
        (.cost.total_lines_added // 0),
        (.cost.total_lines_removed // 0),
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

if [ -z "$current_dir" ] && [ -z "$model_name" ]; then
    current_dir=$(echo "$stdin_data" | jq -r '.workspace.current_dir // .cwd // "unknown"' 2>/dev/null)
    model_name=$(echo "$stdin_data" | jq -r '.model.display_name // "Unknown"' 2>/dev/null)
    lines_added=$(echo "$stdin_data" | jq -r '(.cost.total_lines_added // 0)' 2>/dev/null)
    lines_removed=$(echo "$stdin_data" | jq -r '(.cost.total_lines_removed // 0)' 2>/dev/null)
    duration_ms=$(echo "$stdin_data" | jq -r '(.cost.total_duration_ms // 0)' 2>/dev/null)
    ctx_used=""
    cache_pct="0"
    : "${current_dir:=unknown}"
    : "${model_name:=Unknown}"
    : "${lines_added:=0}"
    : "${lines_removed:=0}"
    : "${duration_ms:=0}"
fi

if cd "$current_dir" 2>/dev/null; then
    git_branch=$(git -c core.useBuiltinFSMonitor=false branch --show-current 2>/dev/null)
    git_root=$(git -c core.useBuiltinFSMonitor=false rev-parse --show-toplevel 2>/dev/null)
fi

# Fallback: try PWD if current_dir didn't yield a git root
if [ -z "$git_root" ]; then
    git_branch=$(git -c core.useBuiltinFSMonitor=false branch --show-current 2>/dev/null)
    git_root=$(git -c core.useBuiltinFSMonitor=false rev-parse --show-toplevel 2>/dev/null)
fi

if [ -n "$git_root" ]; then
    repo_name=$(basename "$git_root")
    if [ "$current_dir" = "$git_root" ]; then
        folder_name="$repo_name"
    else
        folder_name=$(basename "$current_dir")
    fi
else
    folder_name=$(basename "$current_dir")
fi

# illu status
illu_segment=""
status_file=""
if [ -n "$git_root" ] && [ -f "$git_root/.illu/status" ]; then
    status_file="$git_root/.illu/status"
elif [ -f "$current_dir/.illu/status" ]; then
    status_file="$current_dir/.illu/status"
elif [ -f "$PWD/.illu/status" ]; then
    status_file="$PWD/.illu/status"
fi
if [ -n "$status_file" ] && [ -f "$status_file" ]; then
    illu_status=$(cat "$status_file" 2>/dev/null)
    if [ -n "$illu_status" ]; then
        case "$illu_status" in
            ready)
                illu_segment=$(printf '\033[32m◆\033[0m\033[2m illu\033[0m')
                ;;
            indexing*|refreshing*)
                illu_segment=$(printf '\033[33m◆\033[0m \033[33millu: %s\033[0m' "$illu_status")
                ;;
            fetching*)
                illu_segment=$(printf '\033[36m◆\033[0m \033[36millu: %s\033[0m' "$illu_status")
                ;;
            *)
                illu_segment=$(printf '\033[36m◆\033[0m illu: %s' "$illu_status")
                ;;
        esac
    fi
fi

# Colors
C='\033[36m'       # Cyan (accent)
W='\033[97m'       # Bright white
D='\033[2m'        # Dim
R='\033[0m'        # Reset

# Progress bar
progress_bar=""
bar_width=10

if [ -n "$ctx_used" ] && [ "$ctx_used" != "null" ]; then
    filled=$((ctx_used * bar_width / 100))
    empty=$((bar_width - filled))

    if [ "$ctx_used" -lt 50 ]; then
        bar_color='\033[36m'   # Cyan
    elif [ "$ctx_used" -lt 80 ]; then
        bar_color='\033[33m'   # Amber
    else
        bar_color='\033[31m'   # Red
    fi

    progress_bar="${bar_color}"
    for ((i=0; i<filled; i++)); do
        progress_bar="${progress_bar}▰"
    done
    progress_bar="${progress_bar}${D}"
    for ((i=0; i<empty; i++)); do
        progress_bar="${progress_bar}▱"
    done
    progress_bar="${progress_bar}${R}"

    ctx_pct="${bar_color}${ctx_used}%${R}"
else
    ctx_pct=""
fi

# Session duration
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
else
    session_time=""
fi

# Model name — strip prefix, lowercase
short_model=$(echo "$model_name" | sed -E 's/Claude [0-9.]+ //; s/^Claude //' | tr '[:upper:]' '[:lower:]')

# Separator
SEP="${D} · ${R}"

# Assemble
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

if [ -n "$illu_segment" ]; then
    line="${line}$(printf '%b%b' "$SEP" "$illu_segment")"
fi

printf '%b' "$line"
