# dev — Session Manager

`dev` is a Rust CLI that manages persistent tmux sessions for development projects. It auto-discovers projects, creates sessions with configurable layouts, and integrates directly with the chops web UI for remote session management.

## Installation

Build and install from the chops workspace:

```bash
cargo build --release -p dev-cli
cp target/release/dev ~/.local/bin/dev
```

Requires `tmux` to be installed.

## Quick Start

```bash
dev                     # Interactive picker — choose from sessions and projects
dev chops               # Open (or attach to) the chops project
dev claude chops         # Open with claude+shell split layout
dev list                # JSON output of sessions and projects (used by web UI)
```

## Commands

### Interactive picker

```bash
dev
```

Shows all active sessions and available projects. Uses `fzf` if installed, otherwise falls back to a numbered list. Select a session to attach, or a project to create a new session.

### Open a project

```bash
dev <project>
dev claude <project>
```

Resolves the project name, creates a tmux session if one doesn't exist, and attaches. With `claude`, forces the claude+shell layout regardless of config.

Project names are matched in order:
1. Exact display name match
2. Exact directory basename match
3. Substring match (e.g., `chop` matches `chops`)

### Headless session management

These commands are used by the web UI and scripts — they don't attach to the session.

```bash
dev list                        # JSON: active sessions + available projects
dev start <project>             # Create a detached session
dev start <project> claude      # Create with specific layout
dev stop <session>              # Kill a session
```

`dev list` output format:

```json
{
  "sessions": [
    {
      "name": "chops",
      "pane_count": 2,
      "attached": true,
      "last_activity": 1775582804,
      "layout": "claude"
    }
  ],
  "projects": [
    {
      "name": "dotfiles",
      "path": "/home/user/.local/share/chezmoi",
      "layout": "claude",
      "host": null
    }
  ]
}
```

### Layout management

```bash
dev layout              # Show current layout (inside tmux)
dev layout claude       # Add a claude pane to the current session
```

Transforms a single-pane session into a claude+shell split. Only works from inside a tmux session.

### Session lifecycle

```bash
dev kill <name>         # Kill a specific session
dev kill-all            # Kill all sessions (with confirmation prompt)
dev detach              # Detach from current tmux session
```

## Layouts

| Layout | Panes | Description |
|--------|-------|-------------|
| `default` | 1 | Single shell pane in the project directory |
| `claude` | 2 | Vertical split — `claude` (left) + shell (right) |

The plugin-runner targets panes by name:
- `session:1.1` = claude pane (left, in claude layout)
- `session:1.2` = shell pane (right, in claude layout)
- `session:1.1` = the only pane (in default layout)

## Configuration

Per-project settings in `~/.config/dev/config`:

```ini
default_layout=default

# Simple layout override
chops=claude

# Custom directory (expands ~)
dotfiles=claude:~/.local/share/chezmoi

# Remote host — SSH forwards automatically
manta-deploy=claude@myserver

# All options combined
myproject=claude:/opt/myproject@devbox
```

Format: `project=layout[:path][@host]`

| Part | Required | Description |
|------|----------|-------------|
| `layout` | Yes | `default` or `claude` |
| `:path` | No | Custom project directory (overrides ~/Projects discovery) |
| `@host` | No | SSH hostname for remote forwarding |

### Remote projects

If a project has `@host` and the local hostname doesn't match, `dev` automatically SSHs to that host and runs `dev` there. This works for open, kill, and the interactive picker. Remote-only projects (not found locally) still appear in the picker.

## Project discovery

Projects are auto-discovered from `~/Projects/` by scanning up to 3 levels deep for directories containing `.git`.

```
~/Projects/
├── chops/           → "chops"
├── org-a/
│   └── shared/      → "org-a/shared"  (collision handling)
├── org-b/
│   └── shared/      → "org-b/shared"  (collision handling)
└── myproject/       → "myproject"
```

When two projects share the same basename, the relative path is used as the display name to avoid ambiguity.

Custom-path entries from config are merged into the project list.

## Web UI integration

The web dashboard at `https://<host>:8443` uses `dev-lib` (the Rust library) directly — no subprocess calls. The session dropdown, start/stop buttons, and terminal viewer all work through this integration.

| Endpoint | Method | Action |
|----------|--------|--------|
| `/api/sessions` | GET | Calls `DevManager::list()` — returns sessions + projects |
| `/api/sessions/start?project=<name>` | POST | Calls `DevManager::start()` — creates detached session |
| `/api/sessions/stop?session=<name>` | POST | Calls `DevManager::stop()` — kills session |
| `/api/sessions/switch?session=<name>` | POST | Updates ttyd state file for terminal viewer |

## Architecture

`dev` is split into two crates:

- **`dev-lib`** — Pure library with no terminal I/O. Contains config parsing, project discovery, tmux operations (behind a trait for testability), and the `DevManager` API.
- **`dev-cli`** — Thin binary that handles interactive features (picker, SSH forwarding, attach/switch, colored output).

The `web-ui` and `agent-core` crates depend on `dev-lib` directly. This means session management works even if the `dev` binary isn't installed — the web UI calls library functions, not a subprocess.

```
dev-cli ──→ dev-lib      (CLI binary)
web-ui  ──→ dev-lib      (HTTP API)
agent-core ──→ dev-lib   (project discovery for voice commands)
```

## Tmux keybindings

These are configured in `~/.tmux.conf` (managed by chezmoi):

```
prefix = C-a
C-a |       Split horizontally
C-a -       Split vertically
C-a h/j/k/l Navigate panes (vim-style)
C-a r       Reload config
C-a d       Detach from session
```

Sessions are automatically saved every 15 minutes via tmux-continuum and restored on tmux server start via tmux-resurrect.
