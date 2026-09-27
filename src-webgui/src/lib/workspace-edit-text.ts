export type EditPosition = { line: number; character: number }
export type TextEdit = { range: { start: EditPosition; end: EditPosition }; newText: string }

/** LSP defaults to UTF-16 positions, matching JavaScript string offsets. */
export function applyWorkspaceTextEdits(text: string, edits: TextEdit[]): string {
  const lines = text.split('\n')
  const starts: number[] = []
  let total = 0
  for (const line of lines) { starts.push(total); total += line.length + 1 }
  const offset = (p: EditPosition) => {
    if (!Number.isInteger(p.line) || !Number.isInteger(p.character) || p.line < 0 || p.line >= lines.length || p.character < 0 || p.character > lines[p.line].length) throw new Error('Language edit is outside the current document')
    return starts[p.line] + p.character
  }
  const spans = edits.map((e, order) => ({ start: offset(e.range.start), end: offset(e.range.end), text: e.newText.replace(/\r\n?/g, '\n'), order }))
    .sort((a, b) => a.start - b.start || a.end - b.end || a.order - b.order)
  let end = 0
  for (const span of spans) {
    if (span.end < span.start || span.start < end) throw new Error('Language edits overlap; no files were changed')
    end = span.end
  }
  let result = text
  for (const span of spans.reverse()) result = result.slice(0, span.start) + span.text + result.slice(span.end)
  return result
}
