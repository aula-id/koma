import { useEffect, useRef, useState } from 'react'
import Lottie from 'lottie-react'
import animations from 'virtual:lottie-animations'

// The heavy leaf: the actual Lottie player + the bundled animation JSON (kept
// in its own lazy chunk by CatMascot so lottie-react + the animations only
// ship in a separate chunk, off the critical initial paint). Owns which
// random cat is currently showing; picks a NEW random one (never repeating
// the current pick) whenever `swapTrigger` changes — Composer bumps that
// counter once per submit. The player itself never remounts on a swap, so
// the loop keeps running smoothly across cats.
export default function CatMascotLottie({ swapTrigger }: { swapTrigger: number }) {
  const [idx, setIdx] = useState(() => Math.floor(Math.random() * animations.length))
  const mounted = useRef(false)

  useEffect(() => {
    // Skip the mount-time fire — the initial pick above already handles it.
    if (!mounted.current) {
      mounted.current = true
      return
    }
    if (animations.length <= 1) return
    setIdx((prev) => {
      let next = Math.floor(Math.random() * animations.length)
      while (next === prev) next = Math.floor(Math.random() * animations.length)
      return next
    })
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [swapTrigger])

  if (animations.length === 0) throw new Error('no lottie animations bundled')

  return <Lottie animationData={animations[idx]} loop autoplay className="h-full w-full" />
}
