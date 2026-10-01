import { createContext, useContext, useState } from 'react'
import type { DesignToken } from '../../lib/design'

export const DesignModeContext = createContext('light')

function activeValue(token: DesignToken, mode: string): string {
  return token.values[mode] ?? Object.values(token.values)[0] ?? ''
}

function otherModes(token: DesignToken, mode: string): string {
  return Object.entries(token.values)
    .filter(([name]) => name !== mode)
    .map(([name, value]) => `${name} ${value}`)
    .join(' · ')
}

export function TokenMenu({
  tokens,
  selected,
  onPick,
  allowNone = false,
}: {
  tokens: DesignToken[]
  selected?: string | null
  onPick: (name: string | null) => void
  allowNone?: boolean
}) {
  const mode = useContext(DesignModeContext)
  const [query, setQuery] = useState('')
  const needle = query.trim().toLowerCase()
  const rows = tokens.filter((token) => {
    if (!needle) return true
    if (token.name.toLowerCase().includes(needle)) return true
    return Object.values(token.values).some((value) => value.toLowerCase().includes(needle))
  })
  return (
    <div className="flex flex-col gap-1">
      <input
        value={query}
        onChange={(event) => setQuery(event.target.value)}
        placeholder="Search tokens"
        aria-label="Search tokens"
        className="h-7 rounded border border-koma-border bg-koma-bg px-1.5 text-[12px] text-koma-fg outline-none focus:border-koma-fg/40"
      />
      {allowNone ? (
        <button
          type="button"
          className={`flex h-7 w-full items-center px-1.5 text-left text-[12px] ${selected ? 'text-koma-dim hover:bg-koma-hover' : 'bg-koma-hover text-koma-fg'}`}
          onClick={() => onPick(null)}
        >
          None
        </button>
      ) : null}
      {!tokens.length ? <p className="px-1.5 text-[11px] text-koma-dim">No tokens</p> : null}
      {tokens.length && !rows.length ? <p className="px-1.5 text-[11px] text-koma-dim">No matches</p> : null}
      <div className="flex max-h-40 flex-col gap-0.5 overflow-y-auto">
        {rows.map((token) => {
          const value = activeValue(token, mode)
          const rest = otherModes(token, mode)
          const swatch = token.kind === 'color' && value.startsWith('#') ? value : ''
          return (
            <button
              key={token.name}
              type="button"
              title={token.name}
              aria-pressed={selected === token.name}
              onClick={() => onPick(token.name)}
              className={`flex items-center gap-2 rounded px-1.5 py-1 text-left ${selected === token.name ? 'bg-koma-accent/20 text-koma-accent' : 'text-koma-fg hover:bg-koma-hover'}`}
            >
              {swatch ? <span className="h-3.5 w-3.5 flex-none rounded-full border border-koma-border" style={{ background: swatch }} /> : null}
              <span className="min-w-0 flex-1">
                <span className="block truncate text-[12px]">{token.name}</span>
                <span className="block truncate text-[11px] text-koma-dim">{value}{rest ? ` · ${rest}` : ''}</span>
              </span>
            </button>
          )
        })}
      </div>
    </div>
  )
}
