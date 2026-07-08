import { Component, Suspense, lazy, type ReactNode } from 'react'

// Waiting indicator shown in the empty gap above the composer while the turn is
// cooking but NO assistant output has streamed yet (see ChatView's
// `working && !showLive` gate). A playful random Lottie cat; degrades to a
// simple pulse if the animations fail to bundle or the player throws.

const WaitingLottie = lazy(() => import('./WaitingLottie'))

// Bare CSS pulse — the graceful fallback (no wasm, no deps). Also the Suspense
// fallback while the lottie chunk loads on the first wait.
function Pulse() {
  return (
    <div className="flex items-center justify-center py-6" aria-label="Working">
      <div className="h-10 w-10 animate-pulse rounded-full bg-koma-accent/30" />
    </div>
  )
}

// Isolates any lottie-react runtime failure (e.g. the webview can't render a
// given animation) so it falls back to the pulse instead of crashing the chat.
class LottieBoundary extends Component<{ children: ReactNode }, { failed: boolean }> {
  state = { failed: false }
  static getDerivedStateFromError() {
    return { failed: true }
  }
  render() {
    return this.state.failed ? <Pulse /> : this.props.children
  }
}

export function WaitingIndicator() {
  return (
    <LottieBoundary>
      <Suspense fallback={<Pulse />}>
        <WaitingLottie />
      </Suspense>
    </LottieBoundary>
  )
}
