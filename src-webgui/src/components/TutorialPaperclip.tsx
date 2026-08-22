// Pixel paperclip mascot for the Tutorial tab input.
// Pure CSS/div pixels — theme-aware via currentColor (text-koma-accent).

import { useEffect, useState } from 'react'

export type PaperclipMood = 'idle' | 'think' | 'point' | 'wave'

type Props = {
  mood?: PaperclipMood
  onClick?: () => void
  className?: string
  title?: string
}

// 12×14 bitmap, 1 = filled. Classic sideways clip silhouette (very small).
const FRAMES: number[][][] = [
  // idle
  [
    [0,0,0,1,1,1,0,0,0,0,0,0],
    [0,0,1,0,0,0,1,0,0,0,0,0],
    [0,0,1,0,0,0,1,0,0,0,0,0],
    [0,0,1,0,0,0,1,1,1,0,0,0],
    [0,0,1,0,0,0,0,0,0,1,0,0],
    [0,0,1,0,0,0,0,0,0,1,0,0],
    [0,0,1,0,0,0,0,0,0,1,0,0],
    [0,0,1,0,0,0,0,0,0,1,0,0],
    [0,0,0,1,0,0,0,0,1,0,0,0],
    [0,0,0,0,1,1,1,1,0,0,0,0],
    [0,0,0,0,0,0,0,0,0,0,0,0],
    [0,0,0,0,0,0,0,0,0,0,0,0],
    [0,0,0,0,0,0,0,0,0,0,0,0],
    [0,0,0,0,0,0,0,0,0,0,0,0],
  ],
  // bounce / think (shifted up 1)
  [
    [0,0,0,1,1,1,0,0,0,0,0,0],
    [0,0,1,0,0,0,1,0,0,0,0,0],
    [0,0,1,0,0,0,1,1,1,0,0,0],
    [0,0,1,0,0,0,0,0,0,1,0,0],
    [0,0,1,0,0,0,0,0,0,1,0,0],
    [0,0,1,0,0,0,0,0,0,1,0,0],
    [0,0,1,0,0,0,0,0,0,1,0,0],
    [0,0,0,1,0,0,0,0,1,0,0,0],
    [0,0,0,0,1,1,1,1,0,0,0,0],
    [0,0,0,0,0,0,0,0,0,0,0,0],
    [0,0,0,0,0,0,0,0,0,0,0,0],
    [0,0,0,0,0,0,0,0,0,0,0,0],
    [0,0,0,0,0,0,0,0,0,0,0,0],
    [0,0,0,0,0,0,0,0,0,0,0,0],
  ],
  // point (lean right)
  [
    [0,0,0,0,1,1,1,0,0,0,0,0],
    [0,0,0,1,0,0,0,1,0,0,0,0],
    [0,0,0,1,0,0,0,1,0,0,0,0],
    [0,0,0,1,0,0,0,1,1,1,0,0],
    [0,0,0,1,0,0,0,0,0,0,1,0],
    [0,0,0,1,0,0,0,0,0,0,1,0],
    [0,0,0,1,0,0,0,0,0,0,1,0],
    [0,0,0,1,0,0,0,0,0,0,1,0],
    [0,0,0,0,1,0,0,0,0,1,0,0],
    [0,0,0,0,0,1,1,1,1,0,0,0],
    [0,0,0,0,0,0,0,0,0,0,0,0],
    [0,0,0,0,0,0,0,0,1,1,1,1],
    [0,0,0,0,0,0,0,0,0,0,0,1],
    [0,0,0,0,0,0,0,0,0,0,0,0],
  ],
]

export function TutorialPaperclip({ mood = 'idle', onClick, className = '', title }: Props) {
  const [frame, setFrame] = useState(0)

  useEffect(() => {
    if (mood === 'idle') {
      setFrame(0)
      return
    }
    if (mood === 'point') {
      setFrame(2)
      return
    }
    // think / wave — alternate 0/1
    setFrame(1)
    const t = window.setInterval(() => {
      setFrame((f) => (f === 0 ? 1 : 0))
    }, mood === 'think' ? 280 : 180)
    return () => window.clearInterval(t)
  }, [mood])

  const grid = FRAMES[frame] ?? FRAMES[0]
  const px = 2

  return (
    <button
      type="button"
      onClick={onClick}
      title={title ?? 'Tutorial coach'}
      aria-label="Tutorial coach"
      className={`relative flex h-10 w-10 flex-none items-center justify-center rounded-md text-koma-accent transition hover:bg-koma-hover ${className}`}
    >
      <span
        className="block"
        style={{
          width: 12 * px,
          height: 14 * px,
          position: 'relative',
        }}
        aria-hidden
      >
        {grid.map((row, y) =>
          row.map((cell, x) =>
            cell ? (
              <span
                key={`${x}-${y}`}
                className="absolute bg-current"
                style={{
                  left: x * px,
                  top: y * px,
                  width: px,
                  height: px,
                  imageRendering: 'pixelated',
                }}
              />
            ) : null,
          ),
        )}
      </span>
    </button>
  )
}
