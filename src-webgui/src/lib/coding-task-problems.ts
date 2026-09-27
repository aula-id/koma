export type TaskProblem = { path: string; line: number; column: number; message: string }
/** Parse common compiler locations without treating terminal text as markup. */
export function taskProblems(output: string): TaskProblem[] {
  const result: TaskProblem[] = [], seen = new Set<string>()
  const clean = output.replace(/\x1b\[[0-?]*[ -/]*[@-~]/g, '')
  for (const line of clean.split(/\r?\n/)) {
    let match: string[] | null = /^\s*(?:-->\s*)?(.+?):(\d+)(?::(\d+))?(?::|\s|$)\s*(.*)$/.exec(line)
    if (!match) { const ts = /^\s*(.+?)\((\d+),(\d+)\):\s*(.*)$/.exec(line); if (ts) match = ts }
    if (!match) { const python = /^\s*File "(.+)", line (\d+)(.*)$/.exec(line); if (python) match = [python[0], python[1], python[2], '1', python[3]] }
    if (!match || Number(match[2]) < 1 || !/[./\\]/.test(match[1])) continue
    const path = match[1].replace(/^\s*(?:error|warning):\s*/, ''), key = `${path}:${match[2]}:${match[3] ?? 1}`
    if (seen.has(key)) continue; seen.add(key)
    result.push({ path, line: Number(match[2]), column: Number(match[3] ?? 1), message: match[4] || line.trim() })
    if (result.length >= 200) break
  }
  return result
}
