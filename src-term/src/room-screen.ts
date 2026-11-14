import { mkdirSync, readFileSync, writeFileSync } from "node:fs"
import { join } from "node:path"
import { dataDir } from "./paths.ts"
import { SCREENS, type Screen } from "./room-turns.ts"

/** Under this many columns a split stacks its two panes; wider, they sit side by side. */
export const STACK_AT = 80
/** A stacked pane needs this many rows to be worth showing; with less, the room falls back to one view. */
export const MIN_PANE_ROWS = 6

const FILE = "room-screen"

export function isScreen(word: string): word is Screen {
  return (SCREENS as string[]).includes(word)
}

/** The view the user chose last, kept for every room. The first time it is `combine`. */
export function savedScreen(): Screen {
  try {
    const word = readFileSync(join(dataDir(), FILE), "utf8").trim()
    return isScreen(word) ? word : "combine"
  } catch {
    return "combine"
  }
}

export function saveScreen(screen: Screen): void {
  try {
    mkdirSync(dataDir(), { recursive: true })
    writeFileSync(join(dataDir(), FILE), `${screen}\n`)
  } catch {
    // The choice just is not remembered.
  }
}

/** How a split lays out its two panes, or why it cannot. */
export type Layout = { kind: "side" } | { kind: "stack" } | { kind: "single" }

export function splitLayout(width: number, height: number): Layout {
  if (width > STACK_AT) return { kind: "side" }
  return height >= MIN_PANE_ROWS * 2 + 4 ? { kind: "stack" } : { kind: "single" }
}
