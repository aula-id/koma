type OAuthProv = 'OpenAI' | 'Kilo Code' | 'Anthropic'

const OAUTH_PROVIDERS: OAuthProv[] = ['OpenAI', 'Kilo Code', 'Anthropic']

export function OAuthConnect({ onPick, onCancel }: { onPick: (p: OAuthProv) => void; onCancel: () => void }) {
  return (
    <>
      <div className="flex-1 overflow-auto p-3">
        <div className="mb-2 text-[11px] text-koma-fg opacity-50">Choose a provider to connect</div>
        <div className="flex flex-col gap-1.5">
          {OAUTH_PROVIDERS.map((p) => (
            <button
              key={p}
              onClick={() => onPick(p)}
              className="flex items-center justify-between rounded border border-koma-border px-3 py-2 text-[13px] text-koma-fg transition-colors hover:bg-koma-hover"
            >
              <span>{p}</span>
              <span className="text-[10px] uppercase tracking-wide text-koma-fg opacity-40">
                {p === 'Kilo Code' ? 'device code' : 'browser'}
              </span>
            </button>
          ))}
        </div>
        <div className="mt-3 text-[11px] text-koma-fg opacity-35">Opens the provider's sign-in (stub — no backend yet).</div>
      </div>
      <div className="flex flex-none items-center justify-end border-t border-koma-border px-3 py-2">
        <button
          onClick={onCancel}
          className="rounded px-2.5 py-1 text-[12px] text-koma-fg opacity-70 transition-colors hover:bg-koma-hover hover:opacity-100"
        >
          Cancel
        </button>
      </div>
    </>
  )
}
