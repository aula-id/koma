/**
 * Log an error to the Rust backend's global error log (~/.koma/error.log).
 * Used by the error boundary to persist errors that only occur in the built app.
 */
export function logError(context: string, error: unknown): void {
  const fullContext = `[${context}] ${error instanceof Error ? error.message + '\n' + error.stack : String(error)}`
  // Send to Rust backend via IPC (window.komaIpc is set by the host in routes/index.tsx)
  window.komaIpc?.({ r: 'WriteErrorLog', message: fullContext })
}