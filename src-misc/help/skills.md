# Skills and reusable instructions

## GUI
Skills discovers Global, Project and External skills. Search, inspect and explicitly Load/Unload/Reload for the current session. Create, Upload ZIP, Edit with Koma, duplicate, download and delete are available where ownership allows. Editing or installing does not always mean the active session has reloaded the saved instructions; use the displayed Save/reload outcome. External and built-in content may require duplication before editing.
## TUI
/skill manages loaded instructions. Skill instructions are reusable resources, distinct from model providers, MCP tools and extensions.
## Recovery
Check the current session and skill source. Read save/reload feedback carefully. Users explicitly perform loads, installations and deletions; Help never performs these changes.

## How the agent finds skills

The agent is not given a list of your skills. It asks for one by describing the task, and gets back the few best matches (`skill_find`), then loads the one that fits (`skill_load`) and unloads it when done (`skill_unload`). This keeps the prompt small however many skills you have. To make a skill always visible to the agent, add `pinned: true` to its frontmatter (up to eight are shown). Adding `tags: [auth, token]` helps the search find it.

