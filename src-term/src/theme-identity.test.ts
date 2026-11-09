import { expect, test } from "bun:test"
import { scrollbars, themeFor } from "./theme.ts"

// Memoized history turns compare these by identity. A fresh object per render
// re-rendered every mounted turn and leaked native memory in OpenTUI.
test("themes and scrollbar options keep one identity per palette", () => {
  expect(themeFor("dark")).toBe(themeFor("dark"))
  expect(themeFor(null)).toBe(themeFor("dark"))
  expect(themeFor("light")).not.toBe(themeFor("dark"))
  expect(Object.isFrozen(themeFor("dark"))).toBe(true)
  const theme = themeFor("dark")
  expect(scrollbars(theme)).toBe(scrollbars(theme))
  expect(scrollbars(theme).trackOptions.backgroundColor).toBe(theme.panel)
})
