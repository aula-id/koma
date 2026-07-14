// Shared tool-signature formatting — used by ChatView's tool-call rows and
// ApprovalOverlay's risky-tool modal. Both render a `name(<short args>)`
// preview when the host hasn't projected a pre-formatted `signature`.

// Char-aware truncate with an ellipsis (mirrors transcript.rs `truncate_chars`).
export function truncateChars(s: string, max: number): string {
  const chars = Array.from(s)
  return chars.length <= max ? s : `${chars.slice(0, max - 1).join('')}…`
}

// Salient-arg keys per tool (light port of transcript.rs `tool_signature_inner`)
// — used only when the host doesn't supply a pre-formatted `signature`.
const SALIENT_ARG: Record<string, string> = {
  bash: 'command',
  read: 'path',
  write: 'path',
  edit: 'path',
  grep: 'pattern',
  glob: 'pattern',
  dir_list: 'path',
  task: 'agent',
  recall: 'slug',
}

// Fallback display signature `name(arg)` when the host hasn't projected one.
export function fallbackSignature(name: string, args: string): string {
  let inner = ''
  try {
    const parsed = JSON.parse(args)
    if (parsed && typeof parsed === 'object') {
      const key = SALIENT_ARG[name]
      const val = key != null && parsed[key] != null ? parsed[key] : Object.values(parsed)[0]
      inner = val == null ? '' : String(val)
    }
  } catch {
    inner = args
  }
  inner = inner.replace(/\s+/g, ' ').trim()
  return `${name}(${truncateChars(inner, 60)})`
}
