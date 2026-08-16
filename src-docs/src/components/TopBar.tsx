import { Link } from '@tanstack/react-router'
import { BookOpen, ExternalLink } from 'lucide-react'

export function TopBar() {
  return (
    <header className="flex h-10 items-center justify-between border-b border-koma-border px-4 text-sm">
      <Link to="/" className="flex items-center gap-2 font-bold text-koma-accent no-underline">
        <BookOpen size={14} />
        koma docs
      </Link>
      <nav className="flex items-center gap-4">
        <Link
          to="/docs/overview"
          className="text-koma-dim transition hover:text-koma-fg no-underline"
        >
          Docs
        </Link>
        <a
          href="https://koma.run"
          target="_blank"
          rel="noopener noreferrer"
          className="text-koma-dim transition hover:text-koma-fg no-underline"
        >
          koma.run
        </a>
        <a
          href="https://github.com/aula-id/koma"
          target="_blank"
          rel="noopener noreferrer"
          className="text-koma-dim transition hover:text-koma-fg"
        >
          <ExternalLink size={14} />
        </a>
      </nav>
    </header>
  )
}
