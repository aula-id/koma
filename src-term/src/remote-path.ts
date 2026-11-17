/** Parent of a remote POSIX path. A trailing slash is ignored; the remote OS is unknown so backslash is untouched. */
export function parentOf(p: string): string {
  const trimmed = p.replace(/\/+$/, "")
  return trimmed.split("/").slice(0, -1).join("/") || "/"
}
