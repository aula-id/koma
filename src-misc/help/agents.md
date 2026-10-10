# Agents and tasks

## GUI
Agents lists built-in and custom agent definitions. Custom definitions contain instructions and model choices with global/session scopes; built-ins cannot be deleted. Explorer agent rows open read-only streams and expose background/kill controls for eligible running jobs. Ctrl/Cmd+B backgrounds eligible running sub-agents outside editor bindings.
## TUI
/agents manages definitions. /task with no arguments opens the task viewer; /task <agent> <task> launches a named agent. Agent model choices are separate from the session Main model.
## Recovery
Check agent name, definition scope and usable model/provider. Stopping a task does not undo files it already changed.
