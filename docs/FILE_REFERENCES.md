# File references in the TUI

Type `@` in your prompt to browse files beneath the active session's working directory. Keep typing to filter paths, for example:

```text
Review @src/
Explain @README
Compare @docs/notes
```

- Up and Down select a file.
- Enter or Tab inserts the selected path. It does not send your prompt.
- Esc closes the list without deleting your draft or interrupting the agent.
- Filenames containing spaces are quoted automatically, such as `@"docs/design notes.md"`.
- Selecting a file preserves the rest of your prompt, including text after the cursor, and supports undo.

This inserts a path reference. It does not attach or automatically load the file's contents. The agent can read the referenced path when handling your request.

## Scope

The list includes nonignored files and untracked files beneath the session's current directory. It honors `.gitignore` and `.ignore`, including rules that match tracked files. Email addresses do not open the list. Search is case-insensitive substring matching, not fuzzy matching.

Listing runs in the background and is reused while the picker is open. Close and reopen it to refresh files changed on disk. Enumeration stops at 20,000 files, 50,000 visited entries, or two seconds. An incomplete or unavailable list displays a status message rather than claiming a missing match is definitive.

Symlink files and directories are excluded. Native SSH sessions do not list local client files. Ordinary sessions connected to a local Jcode daemon use the selected session's directory, not the daemon process's directory.
