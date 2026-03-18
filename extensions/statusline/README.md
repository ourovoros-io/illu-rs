# illu statusline extension

Shows illu's real-time indexing status in your Claude Code statusline.

## What it looks like

```
▸ opus · my-project › main  ▰▰▰▱▱▱▱▱▱▱ 28%  ◆ illu
```

When illu is actively working:

```
▸ opus · my-project › main  ▰▰▰▱▱▱▱▱▱▱ 28%  ◆ illu: indexing ▸ parsing [5/19] ▰▰▱▱▱▱▱▱
▸ opus · my-project › main  ▰▰▰▱▱▱▱▱▱▱ 28%  ◆ illu: indexing ▸ refs [12/40] ▰▰▰▱▱▱▱▱
▸ opus · my-project › main  ▰▰▰▱▱▱▱▱▱▱ 28%  ◆ illu: fetching docs ▸ 3/8 ▰▰▰▱▱▱▱▱
```

## Status indicators

| Color | Symbol | Meaning |
|-------|--------|---------|
| Green | `◆ illu` | Ready — index is up to date |
| Yellow | `◆ illu: indexing ▸ ... ▰▰▱▱` | Indexing source files or extracting refs |
| Yellow | `◆ illu: refreshing ▸ ... ▰▰▱▱` | Re-indexing changed files |
| Cyan | `◆ illu: fetching docs ▸ ... ▰▰▱▱` | Fetching dependency documentation |

## Installation

### Option 1: Standalone (illu status only)

```bash
cp extensions/statusline/illu-statusline.sh ~/.claude/illu-statusline.sh
chmod +x ~/.claude/illu-statusline.sh
```

In `~/.claude/settings.json`:

```json
{
  "statusLine": {
    "command": "~/.claude/illu-statusline.sh"
  }
}
```

### Option 2: Combined (full statusline + illu)

Includes model name, repo, branch, context usage, session time, cache hit rate, and illu status.

```bash
cp extensions/statusline/combined-statusline.sh ~/.claude/statusline.sh
chmod +x ~/.claude/statusline.sh
```

In `~/.claude/settings.json`:

```json
{
  "statusLine": {
    "command": "~/.claude/statusline.sh"
  }
}
```

### Option 3: Add illu to your existing statusline

Append this snippet to your existing statusline script:

```bash
# Read illu status from repo
git_root=$(git rev-parse --show-toplevel 2>/dev/null)
status_file="${git_root:-.}/.illu/status"
if [ -f "$status_file" ]; then
    illu_status=$(cat "$status_file")
    case "$illu_status" in
        ready)       printf '  \033[32m◆\033[0m\033[2m illu\033[0m' ;;
        index*|ref*) printf '  \033[33m◆ illu: %s\033[0m' "$illu_status" ;;
        fetch*)      printf '  \033[36m◆ illu: %s\033[0m' "$illu_status" ;;
        *)           printf '  \033[36m◆\033[0m illu: %s' "$illu_status" ;;
    esac
fi
```

## Requirements

- `jq` (for parsing Claude Code's JSON input)
- Claude Code with statusline support
