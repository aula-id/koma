import { Link } from '@tanstack/react-router'
import { BookOpen, ExternalLink, Menu, X } from 'lucide-react'

import { useDocsNav } from './DocsNavContext'

export function TopBar() {
  const { open, toggleNav } = useDocsNav()

  return (
    <header className="relative z-50 flex h-14 flex-none items-center justify-between border-b border-koma-border px-4 text-[0.8125rem] sm:px-6">
      <div className="flex min-w-0 items-center gap-2">
        {/* Docs tree toggle — mobile / tablet only; desktop keeps persistent sidebar */}
        <button
          type="button"
          onClick={toggleNav}
          className="inline-flex h-9 w-9 flex-none items-center justify-center rounded-lg text-koma-dim transition-colors hover:bg-koma-hover hover:text-koma-fg md:hidden"
          aria-label={open ? 'Close navigation menu' : 'Open navigation menu'}
          aria-expanded={open}
          aria-controls="docs-mobile-nav"
        >
          {open ? <X size={18} strokeWidth={1.75} /> : <Menu size={18} strokeWidth={1.75} />}
        </button>
        <Link
          to="/welcome"
          className="flex min-w-0 items-center gap-2 font-medium tracking-wide text-koma-fg no-underline transition-colors"
        >
          <BookOpen size={14} strokeWidth={1.75} className="flex-none" />
          <span className="truncate">koma docs</span>
        </Link>
      </div>
      <nav className="flex flex-none items-center gap-0.5 sm:gap-1">
        <Link
          to="/welcome"
          className="hidden rounded-md px-3 py-1.5 text-koma-dim no-underline transition-colors hover:bg-koma-hover hover:text-koma-fg sm:inline"
        >
          Docs
        </Link>
        <a
          href="https://koma.run"
          target="_blank"
          rel="noopener noreferrer"
          className="rounded-md px-2 py-1.5 text-koma-dim no-underline transition-colors hover:bg-koma-hover hover:text-koma-fg sm:px-3"
        >
          koma.run
        </a>
        <a
          href="https://github.com/aula-id/koma"
          target="_blank"
          rel="noopener noreferrer"
          className="rounded-md px-2 py-1.5 text-koma-dim transition-colors hover:bg-koma-hover hover:text-koma-fg sm:px-3"
          aria-label="GitHub"
        >
          <ExternalLink size={14} strokeWidth={1.75} />
        </a>
      </nav>
    </header>
  )
}
