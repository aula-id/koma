import { LANE_COLORS } from '../lib/gitGraphLayout'

// Small, deliberately-deterministic string hash (djb2-ish) — only used to
// pick a stable palette index, never for anything security-sensitive.
function hashStr(s: string): number {
  let h = 0
  for (let i = 0; i < s.length; i++) {
    h = (h * 31 + s.charCodeAt(i)) | 0
  }
  return Math.abs(h)
}

// 1-2 letter initials off the author's NAME ("Jane Doe" -> "JD", "jane" ->
// "J"), falling back to the email's local-part (before `@`) when the name is
// empty/whitespace-only.
function initialsOf(name: string, email: string): string {
  const src = name.trim() || email.split('@')[0] || '?'
  const parts = src.split(/\s+/).filter(Boolean)
  if (parts.length >= 2) return (parts[0][0] + parts[1][0]).toUpperCase()
  return src.slice(0, 2).toUpperCase()
}

const DEFAULT_SIZE = 16

type Props = {
  name: string
  email: string
  size?: number
}

// A small circular THEMED-INITIALS badge for a commit author (GK4b) — no
// network request, no Gravatar: rendering an image fetched off an arbitrary
// author email would be an image-injection vector (and a privacy leak of the
// viewer's IP to whoever owns that email), so this is initials-only by
// design. The background colour is picked deterministically off a hash of
// the author's email (falling back to the name) into the graph's own
// `LANE_COLORS` palette (gitGraphLayout.ts) — that palette was already
// chosen to read on both the light and dark koma themes, so a bespoke
// avatar palette would just be a second copy of the same set of hues.
export function AuthorAvatar({ name, email, size = DEFAULT_SIZE }: Props) {
  const key = email.trim() || name.trim() || '?'
  const color = LANE_COLORS[hashStr(key) % LANE_COLORS.length]
  const label = initialsOf(name, email)
  return (
    <span
      title={email ? `${name} <${email}>` : name}
      style={{ width: size, height: size, backgroundColor: color, fontSize: Math.max(8, size * 0.5) }}
      className="flex flex-none select-none items-center justify-center rounded-full font-semibold leading-none text-white"
    >
      {label}
    </span>
  )
}
