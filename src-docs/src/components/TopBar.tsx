import { Link } from '@tanstack/react-router'
import { BookOpen, ExternalLink } from 'lucide-react'

export function TopBar() {
  return (
    <header className="flex h-14 items-center justify-between border-b border-koma-border px-6 text-[0.8125rem]">
      <Link
        to="/welcome"
        className="flex items-center gap-2 font-medium tracking-wide text-koma-fg no-underline transition-colors hover:text-koma-fg"
      >
        <BookOpen size={14} strokeWidth={1.75} />
        koma docs
      </Link>
      <nav className="flex items-center gap-1">
        <Link
          to="/welcome"
          className="rounded-md px-3 py-1.5 text-koma-dim no-underline transition-colors hover:bg-koma-hover hover:text-koma-fg"
        >
          Docs
        </Link>
        <a
          href="https://koma.run"
          target="_blank"
          rel="noopener noreferrer"
          className="rounded-md px-3 py-1.5 text-koma-dim no-underline transition-colors hover:bg-koma-hover hover:text-koma-fg"
        >
          koma.run
        </a>
        <a
          href="https://github.com/aula-id/koma"
          target="_blank"
          rel="noopener noreferrer"
          className="rounded-md px-3 py-1.5 text-koma-dim transition-colors hover:bg-koma-hover hover:text-koma-fg"
          aria-label="GitHub"
        >
          <ExternalLink size={14} strokeWidth={1.75} />
        </a>
      </nav>
    </header>
  )
}
