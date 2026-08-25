# Prompt suggestions

Prompt suggestions are optional. When enabled, Jcode uses an additional lightweight model request after each successful interactive assistant turn and renders the result as ghost text in an empty TUI composer.

Enable them in `~/.jcode/config.toml`:

```toml
[prompt_suggestions]
enabled = true
```

Press `Tab` or `Right Arrow` to copy visible ghost text into the composer without submitting it. Existing key behavior is unchanged when no suggestion is visible.

Optional settings:

```toml
[prompt_suggestions]
enabled = true
model = "gpt-5.5-mini"
reasoning_effort = "none"
max_chars = 240
acceptance_keys = ["tab", "right_arrow"]
```

Suggestions remain disabled in non-interactive, debug, and scripted flows. Workspace overrides can selectively enable, disable, or tune them:

```toml
[prompt_suggestions.workspaces."/volumes/1tb/projects/jcode"]
enabled = true
```

Workspace keys are normalized to lowercase and omit trailing path separators.
