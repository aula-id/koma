import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from 'react'

type DocsNavContextValue = {
  open: boolean
  openNav: () => void
  closeNav: () => void
  toggleNav: () => void
}

const DocsNavContext = createContext<DocsNavContextValue | null>(null)

export function DocsNavProvider({ children }: { children: ReactNode }) {
  const [open, setOpen] = useState(false)

  const openNav = useCallback(() => setOpen(true), [])
  const closeNav = useCallback(() => setOpen(false), [])
  const toggleNav = useCallback(() => setOpen((v) => !v), [])

  // Escape closes the mobile drawer
  useEffect(() => {
    if (!open) return
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setOpen(false)
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [open])

  // Lock body scroll while the drawer is open (mobile)
  useEffect(() => {
    if (!open) return
    const prev = document.body.style.overflow
    document.body.style.overflow = 'hidden'
    return () => {
      document.body.style.overflow = prev
    }
  }, [open])

  // Desktop resize: drop drawer state so it doesn't stick after rotating to landscape md+
  useEffect(() => {
    const mq = window.matchMedia('(min-width: 768px)')
    const onChange = () => {
      if (mq.matches) setOpen(false)
    }
    mq.addEventListener('change', onChange)
    return () => mq.removeEventListener('change', onChange)
  }, [])

  const value = useMemo(
    () => ({ open, openNav, closeNav, toggleNav }),
    [open, openNav, closeNav, toggleNav],
  )

  return <DocsNavContext.Provider value={value}>{children}</DocsNavContext.Provider>
}

export function useDocsNav(): DocsNavContextValue {
  const ctx = useContext(DocsNavContext)
  if (!ctx) {
    throw new Error('useDocsNav must be used within DocsNavProvider')
  }
  return ctx
}
