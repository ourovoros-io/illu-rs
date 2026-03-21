# illu statusline extension

> **Preferred:** Run `illu-rs install` — it installs the statusline automatically to `~/.illu/statusline.sh` and configures Claude Code. The files below are for manual setup or customization.

Shows illu's real-time indexing status in your Claude Code statusline.

## What it looks like

```
▸ opus · my-project › main  ▰▰▰▱▱▱▱▱▱▱ 28% · 4m12s  ◆ illu
```

When illu is actively working:

```
▸ opus · my-project › main  ▰▰▰▱▱▱▱▱▱▱ 28% · 4m12s  ◆ illu: indexing ▸ parsing [5/19]
▸ opus · my-project › main  ▰▰▰▱▱▱▱▱▱▱ 28% · 4m12s  ◆ illu: indexing ▸ refs [12/40]
▸ opus · my-project › main  ▰▰▰▱▱▱▱▱▱▱ 28% · 4m12s  ◆ illu: fetching docs ▸ 3/8
```

## Status indicators

| Color | Symbol | Meaning |
|-------|--------|---------|
| Green | `◆ illu` | Ready — index is up to date |
| Yellow | `◆ illu: indexing ▸ ...` | Indexing source files or extracting refs |
| Yellow | `◆ illu: refreshing ▸ ...` | Re-indexing changed files |
| Cyan | `◆ illu: fetching docs ▸ ...` | Fetching dependency documentation |

## Manual installation

### Option 1: Standalone (illu status only)

```bash
cp extensions/statusline/illu-statusline.sh ~/.claude/illu-statusline.sh
chmod +x ~/.claude/illu-statusline.sh
```

In `~/.claude/settings.json`:

```json
{
  "statusLine": {
    "type": "command",
    "command": "~/.claude/illu-statusline.sh"
  }
}
```

### Option 2: Combined (full statusline + illu)

```bash
cp extensions/statusline/combined-statusline.sh ~/.claude/statusline.sh
chmod +x ~/.claude/statusline.sh
```

In `~/.claude/settings.json`:

```json
{
  "statusLine": {
    "type": "command",
    "command": "~/.claude/statusline.sh"
  }
}
```

### Option 3: Add illu to your existing statusline

Append this snippet to your existing statusline script:

```bash
# Read illu status from repo
git_root=$(git rev-parse --show-toplevel 2>/dev/null)
status_file=""
if [ -n "$git_root" ] && [ -f "$git_root/.illu/status" ]; then
    status_file="$git_root/.illu/status"
elif [ -f "$PWD/.illu/status" ]; then
    status_file="$PWD/.illu/status"
fi
if [ -n "$status_file" ]; then
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
- `git` (for detecting repo root)
- Claude Code with statusline support
