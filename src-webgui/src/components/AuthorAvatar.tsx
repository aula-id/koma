import { authorColor } from '../lib/authorColor'

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
// design. The background colour comes from the shared `authorColor` helper
// (lib/authorColor.ts) — the SAME mapping GraphBubble's bubble/legend colours
// use (GK5b), so an author's avatar and their bubbles always match.
export function AuthorAvatar({ name, email, size = DEFAULT_SIZE }: Props) {
  const color = authorColor(name, email)
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
