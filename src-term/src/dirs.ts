import { readdir, stat } from "node:fs/promises"
import { homedir } from "node:os"
import { join, resolve } from "node:path"

/** How many folder names the guide shows before "N more". */
export const FOLDER_GUIDE_ROWS = 8

/** Path typed after `/adddir`, or null when that command has no path yet. */
export function addDirArgument(input: string): string | null {
  const match = input.match(/^\/add-?dir(?:\s+)([\s\S]*)$/i)
  if (!match) return null
  const arg = match[1].replace(/\s+$/u, "")
  return arg.length > 0 ? arg : null
}

/**
 * Directory whose children the guide should list.
 * `~/Project` lists Project. A path that is not a directory candidate is unchanged
 * here; the reader drops it when it is not a folder.
 */
export function directoryToList(arg: string, cwd: string, home = homedir(), platform: NodeJS.Platform = process.platform): string | null {
  if (arg === "~" || arg === "~/" || (platform === "win32" && arg === "~\\")) return home
  if (arg.startsWith("~")) {
    if (!tildeSep(arg, platform)) return null
    return resolve(join(home, arg.slice(2)))
  }
  return resolve(cwd || ".", arg)
}

/** Index of the last path separator; backslash counts only on Windows, where it is one. */
export function lastSep(arg: string, platform: NodeJS.Platform = process.platform): number {
  return platform === "win32" ? Math.max(arg.lastIndexOf("/"), arg.lastIndexOf("\\")) : arg.lastIndexOf("/")
}

function tildeSep(arg: string, platform: NodeJS.Platform): boolean {
  return platform === "win32" ? /^~[\\/]/.test(arg) : arg.startsWith("~/")
}

/**
 * Folders to show for a path that may still be incomplete.
 * `~/Project` lists the folders inside Project. `/U` lists the folders in `/`
 * whose names start with `U`, so the guide stays up while typing.
 */
export async function guideFolders(arg: string, cwd: string, home = homedir()): Promise<string[] | null> {
  if (arg.startsWith("~") && arg !== "~" && !tildeSep(arg, process.platform)) return null
  const full = directoryToList(arg, cwd, home)
  if (full && await isDirectory(full)) return listChildFolders(full, "")
  const slash = lastSep(arg)
  const parentArg = slash < 0 ? "." : arg.slice(0, slash + 1)
  const prefix = slash < 0 ? arg : arg.slice(slash + 1)
  const parent = directoryToList(parentArg, cwd, home)
  if (!parent) return null
  return listChildFolders(parent, prefix)
}

async function isDirectory(dir: string): Promise<boolean> {
  try {
    return (await stat(dir)).isDirectory()
  } catch {
    return false
  }
}

/** Child directory names, or null when `dir` cannot be read. Dotfolders stay hidden unless the prefix asks for them. */
export async function listChildFolders(dir: string, prefix = ""): Promise<string[] | null> {
  try {
    const needle = prefix.toLowerCase()
    const entries = await readdir(dir, { withFileTypes: true })
    return entries
      .filter((entry) => entry.isDirectory() && (needle.startsWith(".") || !entry.name.startsWith(".")))
      .map((entry) => entry.name)
      .filter((name) => name.toLowerCase().startsWith(needle))
      .sort((a, b) => a.localeCompare(b, undefined, { sensitivity: "base" }))
  } catch {
    return null
  }
}
