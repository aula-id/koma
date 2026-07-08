import { useState } from 'react'
import Lottie from 'lottie-react'
import animations from 'virtual:lottie-animations'

// The heavy leaf: a random Lottie from public/lottie (extracted + inlined at
// build by vite-plugin-lottie). Lazy-loaded by WaitingIndicator so lottie-react
// + the bundled animation JSON only ship in a separate chunk fetched on the
// first wait. If no animations were bundled, it throws so the boundary in
// WaitingIndicator drops to the pulse fallback.
export default function WaitingLottie() {
  // Random pick, chosen once per mount → one animation per wait.
  const [idx] = useState(() => Math.floor(Math.random() * animations.length))

  if (animations.length === 0) throw new Error('no lottie animations bundled')

  return (
    <div className="flex items-center justify-center py-6">
      <Lottie animationData={animations[idx]} loop autoplay className="h-28 w-28" />
    </div>
  )
}
