# Saved notification history

## GUI
The bell opens Notifications. Select Session or App, search by text and source, or filter severity. Rows are newest first; selecting a row opens details and marks it read. Mark all read affects the selected scope. Clear history requires confirmation.
## TUI
/notification opens searchable history. Arrows select, Enter expands and marks read, Tab switches Session/App, Ctrl+R marks all read, Ctrl+X asks to clear, and Esc returns.
## Storage and recovery
The newest 500 entries per scope are saved. Session history lives in notifications.sqlite in the session directory on the session host. App history belongs to the local installation. Popup dismissal and expiry do not delete history or change read state. Existing sessions begin with empty history; past expired popups cannot be recovered. Entries have no retry, navigation or assistant actions. If storage fails, temporary feedback still works.
