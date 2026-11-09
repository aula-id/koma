import type { BorderSides } from "@opentui/core"
/** Colour roles shared with `view/theme.rs`. */

export interface Theme {
  name: string
  bg: string
  fg: string
  dim: string
  accent: string
  panel: string
  selFg: string
  selBg: string
  success: string
  warn: string
  error: string
  info: string
  heat: [string, string, string, string, string]
}

/** Scrollbar track follows the panel, and the thumb follows dim text. */
type ScrollbarOptions = { trackOptions: { backgroundColor: string; foregroundColor: string } }
const scrollbarCache = new Map<string, ScrollbarOptions>()

/**
 * Scrollbar options, one object per color pair. OpenTUI rebuilds the native
 * scrollbars whenever this prop changes identity, and every render used to
 * pass a fresh object, which leaked native memory on each repaint.
 */
export function scrollbars(theme: Pick<Theme, "panel" | "dim">): ScrollbarOptions {
  const key = `${theme.panel}|${theme.dim}`
  let options = scrollbarCache.get(key)
  if (!options) {
    options = { trackOptions: { backgroundColor: theme.panel, foregroundColor: theme.dim } }
    scrollbarCache.set(key, options)
  }
  return options
}

/** Stable border-side lists. A fresh array per render makes OpenTUI redo the border. */
export const BORDER_LEFT: BorderSides[] = ["left"]
export const BORDER_TOP: BorderSides[] = ["top"]
export const BORDER_BOTTOM: BorderSides[] = ["bottom"]

type Paint = Omit<Theme, "name">

const PALETTES: Record<string, Paint> = {
  dark: {
    bg: "#000000",
    fg: "#e6e6e6",
    dim: "#adadad",
    accent: "#39ff14",
    panel: "#2b2f38",
    selFg: "#000000",
    selBg: "#39ff14",
    success: "#00c853",
    warn: "#ffb43c",
    error: "#ff3c3c",
    info: "#50c8ff",
    heat: ["#232323", "#00783c", "#64a032", "#c88c00", "#dc3232"],
  },
  light: {
    bg: "#fafaf6",
    fg: "#141414",
    dim: "#787878",
    accent: "#008000",
    panel: "#e4e6eb",
    selFg: "#ffffff",
    selBg: "#008000",
    success: "#00963c",
    warn: "#c87800",
    error: "#c82828",
    info: "#1e78c8",
    heat: ["#e1e1e1", "#009646", "#78aa28", "#d28c00", "#d23232"],
  },
  forest: {
    bg: "#333d29",
    fg: "#c2c5aa",
    dim: "#a4ac86",
    accent: "#a68a64",
    panel: "#414833",
    selFg: "#333d29",
    selBg: "#a68a64",
    success: "#656d4a",
    warn: "#936639",
    error: "#b04a34",
    info: "#7a9682",
    heat: ["#414833", "#656d4a", "#a68a64", "#936639", "#b04a34"],
  },
  autumn: {
    bg: "#2e2a20",
    fg: "#f1dca7",
    dim: "#baa587",
    accent: "#ffcb69",
    panel: "#3d3729",
    selFg: "#2e2a20",
    selBg: "#ffcb69",
    success: "#9b9b7a",
    warn: "#e8ac65",
    error: "#c85a3c",
    info: "#7d9b86",
    heat: ["#3d3729", "#797d62", "#baa587", "#e8ac65", "#c85a3c"],
  },
  warm: {
    bg: "#ffe4d5",
    fg: "#5b3a30",
    dim: "#966656",
    accent: "#e0603f",
    panel: "#fbd4b4",
    selFg: "#ffe4d5",
    selBg: "#e0603f",
    success: "#5f8c42",
    warn: "#c87814",
    error: "#c6322d",
    info: "#2e78a8",
    heat: ["#fddcc5", "#f6c492", "#f9b16e", "#f8a174", "#f7907a"],
  },
  "cold symphony": {
    bg: "#e2eafc",
    fg: "#1e345c",
    dim: "#5f78a5",
    accent: "#4a7ce0",
    panel: "#c0d3f9",
    selFg: "#e2eafc",
    selBg: "#4a7ce0",
    success: "#3c966e",
    warn: "#c88c28",
    error: "#cd4646",
    info: "#6a99f1",
    heat: ["#daffef", "#a9f0e5", "#78e0db", "#88aff4", "#6a99f1"],
  },
  winter: {
    bg: "#eff7f6",
    fg: "#2a3a44",
    dim: "#6e8791",
    accent: "#d66ea4",
    panel: "#d1f7f3",
    selFg: "#eff7f6",
    selBg: "#d66ea4",
    success: "#46a078",
    warn: "#c88c28",
    error: "#cd505a",
    info: "#46afcd",
    heat: ["#d1f7f3", "#b2f7ef", "#7bdff2", "#f5c6da", "#f2b5d4"],
  },
  monokai: {
    bg: "#272822",
    fg: "#f8f8f2",
    dim: "#75715e",
    accent: "#f92672",
    panel: "#3e3d32",
    selFg: "#f8f8f2",
    selBg: "#49483e",
    success: "#a6e22e",
    warn: "#fd971f",
    error: "#f92672",
    info: "#66d9ef",
    heat: ["#49483e", "#a6e22e", "#e6db74", "#fd971f", "#f92672"],
  },
  vscode: {
    bg: "#1e1e1e",
    fg: "#d4d4d4",
    dim: "#808080",
    accent: "#569cd6",
    panel: "#252526",
    selFg: "#d4d4d4",
    selBg: "#264f78",
    success: "#6a9955",
    warn: "#dcdcaa",
    error: "#f44747",
    info: "#4fc1ff",
    heat: ["#3a3d41", "#6a9955", "#dcdcaa", "#ce9178", "#f44747"],
  },
  "github dark": {
    bg: "#0d1117",
    fg: "#c9d1d9",
    dim: "#8b949e",
    accent: "#58a6ff",
    panel: "#161b22",
    selFg: "#c9d1d9",
    selBg: "#163356",
    success: "#3fb950",
    warn: "#d29922",
    error: "#f85149",
    info: "#79c0ff",
    heat: ["#161b22", "#3fb950", "#d29922", "#db6d28", "#f85149"],
  },
}

/** Names the daemon may send, in the settings Appearance order. */
export const PALETTE_NAMES = Object.keys(PALETTES)

/** Resolve a snapshot palette name. An unknown name paints dark and keeps its name. */
const themeCache = new Map<string, Theme>()

/**
 * One frozen object per palette name. Callers run this every render, and memoized
 * history turns compare `theme` by identity; a fresh object re-rendered every turn.
 */
export function themeFor(name: string | null | undefined): Theme {
  const requested = name && name.trim().length > 0 ? name : "dark"
  let theme = themeCache.get(requested)
  if (!theme) {
    const paint = PALETTES[requested] ?? PALETTES.dark
    theme = Object.freeze({ name: requested, ...paint })
    themeCache.set(requested, theme)
  }
  return theme
}
