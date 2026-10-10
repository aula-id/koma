export type TerminalShell = { id?: string; label: string }
export type TerminalShellReply = { requestId: string; context: string; shells: TerminalShell[]; error?: string | null }
