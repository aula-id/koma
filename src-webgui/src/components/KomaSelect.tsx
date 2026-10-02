import { ChevronDown } from 'lucide-react'
import { useMemo, type SelectHTMLAttributes } from 'react'
import { luminance } from '../lib/luminance'
import { useKoma } from '../store/koma'

type Props = SelectHTMLAttributes<HTMLSelectElement> & {
  /** Custom chevron; off for very compact controls (e.g. Fix/Hug/Fill). */
  chevron?: boolean
}

/** Native `<select>` with Koma colors and `colorScheme` so the menu matches the theme. */
export function KomaSelect({ className = '', chevron = true, style, disabled, ...props }: Props) {
  const background = useKoma((s) => s.palette.bg)
  const colorScheme = useMemo(() => (luminance(background) < 0.5 ? 'dark' : 'light'), [background])
  const selectClass = [
    'rounded border border-koma-border bg-koma-bg text-koma-fg outline-none focus:border-koma-accent disabled:opacity-40 disabled:pointer-events-none',
    chevron ? 'appearance-none pr-5' : '',
    className,
  ]
    .filter(Boolean)
    .join(' ')
  const select = (
    <select {...props} disabled={disabled} style={{ colorScheme, ...style }} className={selectClass} />
  )
  if (!chevron) return select
  const fullWidth = /\bw-full\b/.test(className)
  return (
    <span className={`relative inline-flex max-w-full align-middle ${fullWidth ? 'min-w-0 w-full flex-1' : ''} ${disabled ? 'opacity-40' : ''}`}>
      {select}
      <ChevronDown size={12} className="pointer-events-none absolute right-1 top-1/2 -translate-y-1/2 text-koma-dim" aria-hidden />
    </span>
  )
}
