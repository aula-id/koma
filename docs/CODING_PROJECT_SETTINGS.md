# Project coding settings

Use **Open Project Coding Settings** in the command palette. Koma creates
`.koma/coding.json` only when missing, then opens it in the normal editor. Existing
files are never overwritten by this command. Save to apply changes to open
editors in that host/workspace. External changes are checked when the window
regains focus. The workspace dropdown and two-pane layout are unchanged.

```json
{
  "version": 1,
  "editor": {
    "tabSize": 2,
    "insertSpaces": true,
    "wordWrap": "on",
    "minimap": false,
    "inlayHints": true,
    "lineNumbers": "on",
    "renderWhitespace": "selection"
  },
  "languages": {
    "go": { "editor": { "tabSize": 4, "insertSpaces": false } }
  },
  "snippets": {
    "typescript": [
      { "prefix": "log", "body": "console.log(${1:value});$0", "description": "Log a value" }
    ],
    "*": [
      { "prefix": "todo", "body": "TODO: ${1:description}$0" }
    ]
  },
  "keybindings": [
    { "key": "Mod+Shift+F", "command": "editor.action.formatDocument" },
    { "key": "Mod+S", "command": "save" }
  ]
}
```

`Mod` is Ctrl on Windows/Linux and Command on macOS. `Shift` and `Alt` are
supported; keys use Monaco names (`Enter`, `Tab`, `F2`, etc.) or a letter/digit.
Bindings apply only in the project editor and refer to registered editor action
IDs, including `koma.rename` and `koma.undoWorkspaceEdit`. Invalid settings are
reported in a toast. No shell commands are executed from editor keybindings.

Snippets use standard tab stops/placeholders; `body` may also be an array of
lines. Language keys are LSP language IDs (`typescriptreact` for TSX,
`javascriptreact` for JSX). `*` snippets apply to every language.

Project `tasks` now configure Run/Build/Test commands; see
[Project tasks](CODING_TASKS.md) for the schema, execution behavior, and native
review cases. Toolchains, debug, structured tests and environment selection
remain reserved for later increments. Format-on-save is also not implemented
yet; use Format Document/Selection explicitly.

The editor now supports `"formatOnSave": true` (default false) and
`"semanticHighlighting": false` (default true). Both accept per-language editor
overrides. Formatting errors leave the file unsaved; disable format-on-save or
repair the language server before retrying. The project configuration document
is exempt so an invalid setting cannot prevent its own repair.
