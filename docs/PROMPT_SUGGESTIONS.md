# Prompt suggestions

Prompt suggestions are enabled by default for compatible interactive clients. Jcode uses an additional lightweight model request after each successful interactive assistant turn and renders the result as ghost text in an empty TUI composer.

Disable them in `~/.jcode/config.toml`:

```toml
[prompt_suggestions]
enabled = false
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

Suggestions require a compatible interactive client. Workspace overrides can selectively enable, disable, or tune them:

```toml
[prompt_suggestions.workspaces."/volumes/1tb/projects/jcode"]
enabled = false
```

Workspace keys are normalized to lowercase and omit trailing path separators.
