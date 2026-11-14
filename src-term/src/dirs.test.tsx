import { expect, test } from "bun:test"
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs"
import { tmpdir } from "node:os"
import { join, parse, resolve } from "node:path"
import { act } from "react"
import { testRender } from "@opentui/react/test-utils"
import { addDirArgument, directoryToList, guideFolders, listChildFolders } from "./dirs.ts"
import { FolderGuide } from "./dirs.tsx"
import { themeFor } from "./theme.ts"

test("the adddir guide reads the path and lists that directory", () => {
  expect(addDirArgument("/adddir")).toBeNull()
  expect(addDirArgument("/adddir ")).toBeNull()
  expect(addDirArgument("/help ~/Project")).toBeNull()
  expect(addDirArgument("/adddir ~/Project")).toBe("~/Project")
  expect(addDirArgument("/add-dir ~/Project")).toBe("~/Project")
  expect(addDirArgument("/adddir /")).toBe("/")
  expect(addDirArgument("/adddir /home")).toBe("/home")
  expect(addDirArgument("/adddir ~/")).toBe("~/")
  expect(addDirArgument("/adddir ./")).toBe("./")
  const work = resolve(tmpdir(), "koma guide work")
  const home = resolve(tmpdir(), "koma guide home")
  const root = parse(work).root
  expect(directoryToList("~/Project", work, home)).toBe(join(home, "Project"))
  expect(directoryToList("~/", work, home)).toBe(home)
  expect(directoryToList("~", work, home)).toBe(home)
  expect(directoryToList("~other", work, home)).toBeNull()
  expect(directoryToList("./", work, home)).toBe(work)
  expect(directoryToList("src", work, home)).toBe(join(work, "src"))
  expect(directoryToList(root, work, home)).toBe(root)
  expect(directoryToList(join(root, "home"), work, home)).toBe(join(root, "home"))
})

test("a directory guide lists folders inside the typed path and skips files", async () => {
  const root = mkdtempSync(join(tmpdir(), "koma-adddir-"))
  const project = join(root, "Project")
  mkdirSync(join(project, "src"), { recursive: true })
  mkdirSync(join(project, "notes"), { recursive: true })
  mkdirSync(join(project, ".hidden"), { recursive: true })
  writeFileSync(join(project, "readme.txt"), "hi")
  try {
    expect(await listChildFolders(project)).toEqual(["notes", "src"])
    expect(await listChildFolders(join(project, "missing"))).toBeNull()
    expect(await guideFolders("Project", root)).toEqual(["notes", "src"])
    expect(await guideFolders("P", root)).toEqual(["Project"])
    expect(await guideFolders("Project/s", root)).toEqual(["src"])
    expect(await guideFolders("Project/z", root)).toEqual([])
    const theme = themeFor("forest")
    const view = await testRender(<FolderGuide draft={`/adddir ${project}/s`} cwd={root} theme={theme} />, { width: 80, height: 16 })
    try {
      // The listing is read from disk, so a slow runner can need longer than a fixed wait.
      let frame = ""
      for (let i = 0; i < 40 && !frame.includes("src"); i++) {
        await act(async () => { await Bun.sleep(50) })
        await act(async () => { await view.waitForVisualIdle() })
        frame = view.captureCharFrame()
      }
      expect(frame).toContain("src")
      expect(frame).not.toContain("notes")
      expect(frame).not.toContain("readme.txt")
      expect(frame).not.toContain(".hidden")
      const quiet = await testRender(<FolderGuide draft="/adddir" cwd={root} theme={theme} />, { width: 80, height: 12 })
      try {
        await act(async () => { await Bun.sleep(80) })
        expect(quiet.captureCharFrame().trim()).toBe("")
      } finally {
        await act(async () => { await quiet.dispose() })
      }
    } finally {
      await act(async () => { await view.dispose() })
    }
  } finally {
    rmSync(root, { recursive: true, force: true })
  }
})
