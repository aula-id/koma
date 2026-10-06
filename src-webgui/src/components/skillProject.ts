/** Workspace the Project skill catalogue uses. Empty when no project is open. */
export function skillProjectRoot(workdirs: readonly string[] | null | undefined, activeRoot: string | null | undefined): string {
  const roots = (workdirs ?? []).filter(Boolean)
  if (activeRoot && roots.includes(activeRoot)) return activeRoot
  return roots[0] ?? ''
}

/** A chat plus a workspace. Global skills stay editable without this. */
export function insideSkillProject(
  sessionId: string | null | undefined,
  workdirs: readonly string[] | null | undefined,
  activeRoot: string | null | undefined,
): boolean {
  return Boolean(sessionId && skillProjectRoot(workdirs, activeRoot))
}
