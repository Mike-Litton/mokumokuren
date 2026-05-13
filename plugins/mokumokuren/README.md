# mokumokuren — Claude Code plugin

Wires [mokumokuren](https://github.com/Mike-Litton/mokumokuren)
(`mmk`) into Claude Code's edit loop so Git-history findings —
HOTSPOT, COUPLING, BUDGET, DRIFT, and the rest — arrive as
`additionalContext` after every Edit / Write and after every
`git commit`.

## What it installs

- **Post-edit hook** (`PostToolUse:Edit|Write`) — runs `mmk review`
  against the working tree. Findings flow into the agent's next-turn
  context.
- **Post-commit hook** (`PostToolUse:Bash(git commit:*)`) — runs
  `mmk session-summary --base <upstream-default> --drift-sessions 5`
  the moment a commit lands.
- **Skill `mmk-on-edit`** — workflow guide for invoking `mmk
  pre-edit`, `mmk review`, `mmk session-summary` at the right
  loop phase.
- **Skill `mmk-findings`** — interpretation guide: per-sensor
  priors, override discipline, reading silence.
- **Agent `mmk-assessor`** — bounded subagent that runs `mmk
  analyze` / `mmk drift` / `mmk pre-edit` and returns a brief
  written assessment. Invoke at the start of a task.

The plugin defaults to advisory output (`--gate none`). For
strict-mode `--gate warn` (hard yield via `decision: "block"`),
override the hook command in your project's `.claude/settings.json`
— see [Strict mode](#strict-mode-gate-warn) below.

## Install

The plugin wires the integration; the `mmk` binary ships separately.

### 1. Install the `mmk` binary

| Audience      | Command                                                                                                                  |
| ------------- | ------------------------------------------------------------------------------------------------------------------------ |
| macOS / Linux | `curl -LsSf https://github.com/Mike-Litton/mokumokuren/releases/latest/download/mokumokuren-installer.sh \| sh`           |
| Windows       | `iwr https://github.com/Mike-Litton/mokumokuren/releases/latest/download/mokumokuren-installer.ps1 \| iex`                |
| From source   | clone the repo, then `cargo install --path mmk-cli --locked`                                                             |

Both `mokumokuren` and `mmk` land on `$PATH`.

### 2. Install the plugin

In Claude Code:

```
/plugin marketplace add Mike-Litton/mokumokuren
/plugin install mokumokuren@mokumokuren-plugins
```

### 3. Verify the install

Edit any tracked file in a Git repo with some history. The
`PostToolUse:Edit` hook should fire and findings (or an `[no
actionable signal]` line) should arrive as `additionalContext` in
the next turn. If you see the "mmk not on PATH" advisory instead,
step 1 didn't put `mmk` on this shell's `PATH` — see below.

## "mmk not on PATH" warning

If you see

> mokumokuren plugin is installed but the `mmk` binary is not on
> PATH. Install: …

…the plugin's hook ran but couldn't find `mmk`. Install the binary
(step 1 above), then restart your Claude Code session so the new
`PATH` is picked up.

## Strict mode (`--gate warn`)

The default hooks emit advisory findings via
`hookSpecificOutput.additionalContext`. To make `mmk` hard-yield
on warn-severity findings (via `decision: "block"`), override the
plugin's `review-on-edit.sh` hook command in your project's
`.claude/settings.json`:

```json
{
  "hooks": {
    "PostToolUse": [
      {
        "matcher": "Edit|Write",
        "hooks": [{ "type": "command", "command": "mmk review --gate warn" }]
      }
    ]
  }
}
```

Project-level hooks layer over plugin hooks — both fire. If you
want strict mode *instead of* advisory, disable the plugin's
post-edit hook through `/plugin` and use only your project hook.

## Uninstall

```
/plugin uninstall mokumokuren@mokumokuren-plugins
```

The `mmk` binary stays on `$PATH`; remove it via your installer
of choice (`cargo uninstall mokumokuren`, the installer's
uninstall step, or `rm $(which mmk)`).

## License

MIT OR Apache-2.0 (matches the parent project).
