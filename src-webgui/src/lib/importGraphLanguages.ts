/**
 * Extension → language mapping for the import graph.
 * Exact match with backend `Lang` enum in linker/graph.rs.
 */

type LangName =
  | 'Rust'
  | 'Python'
  | 'Go'
  | 'Java'
  | 'TypeScript'
  | 'JavaScript'
  | 'Php'
  | 'C'
  | 'Cpp'
  | 'Dart'
  | 'Swift'

const EXT_MAP: Record<string, LangName> = {
  '.rs': 'Rust',
  '.py': 'Python',
  '.go': 'Go',
  '.java': 'Java',
  '.ts': 'TypeScript',
  '.tsx': 'TypeScript',
  '.js': 'JavaScript',
  '.jsx': 'JavaScript',
  '.mjs': 'JavaScript',
  '.cjs': 'JavaScript',
  '.php': 'Php',
  '.c': 'C',
  '.h': 'C',
  '.cpp': 'Cpp',
  '.cc': 'Cpp',
  '.cxx': 'Cpp',
  '.hpp': 'Cpp',
  '.hxx': 'Cpp',
  '.dart': 'Dart',
  '.swift': 'Swift',
}

/** All source languages the linker supports, in backend enum order. */
export const SOURCE_LANGUAGES = [
  'Rust',
  'Python',
  'Go',
  'Java',
  'TypeScript',
  'JavaScript',
  'Php',
  'C',
  'Cpp',
  'Dart',
  'Swift',
] as const

export type SourceLanguage = (typeof SOURCE_LANGUAGES)[number]

/**
 * Map a file path to its linker language, or null if unsupported.
 * Matches the backend `sourceLanguage()` function exactly.
 */
export function sourceLanguage(path: string): SourceLanguage | null {
  const dot = path.lastIndexOf('.')
  if (dot < 0) return null
  const ext = path.slice(dot).toLowerCase()
  return (EXT_MAP[ext] as SourceLanguage | undefined) ?? null
}
