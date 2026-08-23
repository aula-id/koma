// Coding panel slice — workspace file tree + editor document state.
// Merged into the main koma store as `coding` (see store/koma.ts).
// Actions live on KomaState; this module owns the state shape + push reducers.

export type FileTreeEntry = {
  name: string
  path: string
  isDir: boolean
}

export type CodingFileState = {
  content: string | null // null = still loading / unavailable
  savedContent: string | null // content at last save/read
  fingerprint: string // from last FileRead or FileSave reply
  dirty: boolean
  loading: boolean
  saving: boolean
  conflict: boolean // stale save was rejected
  error: string | null
  binary: boolean
  tooLarge: boolean
}

export type DirState = {
  entries: FileTreeEntry[]
  loading: boolean
  error: string | null
}

export type CodingSlice = {
  activeRoot: string | null
  dirs: Record<string, DirState> // key = `${root}:${path}`
  files: Record<string, CodingFileState> // key = `${root}:${path}`
  // Last requestId per root+path for stale-reply rejection.
  _readReq: Record<string, string>
  _treeReq: Record<string, string>
  _sessionGen: number // bumped on session switch
}

export const initialCoding: CodingSlice = {
  activeRoot: null,
  dirs: {},
  files: {},
  _readReq: {},
  _treeReq: {},
  _sessionGen: 0,
}

export function fileKey(root: string, path: string): string {
  return `${root}:${path}`
}

export function mintRequestId(): string {
  return `${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`
}

export function parentDirPath(path: string): string {
  if (!path) return ''
  const idx = path.lastIndexOf('/')
  return idx <= 0 ? '' : path.slice(0, idx)
}

export function baseName(path: string): string {
  const parts = path.split('/').filter(Boolean)
  return parts[parts.length - 1] || path
}

export function emptyFileState(partial?: Partial<CodingFileState>): CodingFileState {
  return {
    content: null,
    savedContent: null,
    fingerprint: '',
    dirty: false,
    loading: false,
    saving: false,
    conflict: false,
    error: null,
    binary: false,
    tooLarge: false,
    ...partial,
  }
}

// Coding push envelope shapes (mirrored from koma.d.ts CodingPush).
export type FileTreePush = {
  k: 'FileTree'
  root: string
  path: string
  requestId: string
  entries: FileTreeEntry[]
  error: string | null
}
export type FileReadPush = {
  k: 'FileRead'
  root: string
  path: string
  requestId: string
  content: string | null
  fingerprint: string
  binary: boolean
  tooLarge: boolean
  error: string | null
}
export type FileSavePush = {
  k: 'FileSave'
  root: string
  path: string
  requestId: string
  fingerprint: string
  error: string | null
}
export type FileCreatePush = {
  k: 'FileCreate'
  root: string
  path: string
  requestId: string
  error: string | null
}
export type FileRenamePush = {
  k: 'FileRename'
  root: string
  oldPath: string
  newPath: string
  requestId: string
  error: string | null
}
export type FileDeletePush = {
  k: 'FileDelete'
  root: string
  path: string
  requestId: string
  error: string | null
}
export type FileWriteBytesPush = {
  k: 'FileWriteBytes'
  root: string
  path: string
  requestId: string
  error: string | null
}
export type FileDownloadBytesPush = {
  k: 'FileDownloadBytes'
  root: string
  path: string
  requestId: string
  bytesB64: string | null
  size: number
  tooLarge: boolean
  error: string | null
}

export type CodingPush =
  | FileTreePush
  | FileReadPush
  | FileSavePush
  | FileCreatePush
  | FileRenamePush
  | FileDeletePush
  | FileWriteBytesPush
  | FileDownloadBytesPush

/** Apply a FileTree push into the coding slice (stale-reply guarded). */
export function reduceFileTree(coding: CodingSlice, env: FileTreePush): CodingSlice {
  const key = fileKey(env.root, env.path)
  if (coding._treeReq[key] && coding._treeReq[key] !== env.requestId) return coding
  return {
    ...coding,
    dirs: {
      ...coding.dirs,
      [key]: {
        entries: env.error ? [] : env.entries ?? [],
        loading: false,
        error: env.error,
      },
    },
  }
}

/** Apply a FileRead push into the coding slice (stale-reply guarded). */
export function reduceFileRead(coding: CodingSlice, env: FileReadPush): CodingSlice {
  const key = fileKey(env.root, env.path)
  if (coding._readReq[key] && coding._readReq[key] !== env.requestId) return coding
  const prev = coding.files[key]
  // Don't clobber local dirty edits with a late re-read unless there was no prior content.
  if (prev?.dirty && prev.content != null && !env.error && !env.binary && !env.tooLarge) {
    return {
      ...coding,
      files: {
        ...coding.files,
        [key]: {
          ...prev,
          loading: false,
          // Keep fingerprint from disk so a later save can detect conflict.
          fingerprint: env.fingerprint || prev.fingerprint,
          error: null,
          binary: false,
          tooLarge: false,
        },
      },
    }
  }
  if (env.error) {
    return {
      ...coding,
      files: {
        ...coding.files,
        [key]: emptyFileState({
          loading: false,
          error: env.error,
          fingerprint: prev?.fingerprint ?? '',
        }),
      },
    }
  }
  if (env.binary || env.tooLarge) {
    return {
      ...coding,
      files: {
        ...coding.files,
        [key]: emptyFileState({
          loading: false,
          binary: env.binary,
          tooLarge: env.tooLarge,
          fingerprint: env.fingerprint || '',
        }),
      },
    }
  }
  const content = env.content ?? ''
  return {
    ...coding,
    files: {
      ...coding.files,
      [key]: emptyFileState({
        content,
        savedContent: content,
        fingerprint: env.fingerprint || '',
        loading: false,
        dirty: false,
      }),
    },
  }
}

/** Apply a FileSave push into the coding slice. */
export function reduceFileSave(coding: CodingSlice, env: FileSavePush): CodingSlice {
  const key = fileKey(env.root, env.path)
  const prev = coding.files[key]
  if (!prev) return coding
  if (env.error) {
    // Conflict / failure: keep dirty content, flag conflict.
    return {
      ...coding,
      files: {
        ...coding.files,
        [key]: {
          ...prev,
          saving: false,
          conflict: true,
          error: env.error,
        },
      },
    }
  }
  const content = prev.content ?? prev.savedContent ?? ''
  return {
    ...coding,
    files: {
      ...coding.files,
      [key]: {
        ...prev,
        content,
        savedContent: content,
        fingerprint: env.fingerprint || prev.fingerprint,
        dirty: false,
        saving: false,
        conflict: false,
        error: null,
      },
    },
  }
}

/** True when `path` is `prefix` itself or a path under `prefix/`. */
export function isPathOrDescendant(path: string, prefix: string): boolean {
  if (prefix === '') return true
  return path === prefix || path.startsWith(prefix + '/')
}

/** Remap a path that lives at/under `oldPath` onto `newPath`. */
export function remapPath(path: string, oldPath: string, newPath: string): string | null {
  if (path === oldPath) return newPath
  if (oldPath !== '' && path.startsWith(oldPath + '/')) {
    return newPath + path.slice(oldPath.length)
  }
  return null
}

/** After create/rename/delete success, drop cached dir listings under the parent so a refresh reloads. */
export function invalidateDir(coding: CodingSlice, root: string, path: string): CodingSlice {
  const parent = parentDirPath(path)
  const key = fileKey(root, parent)
  const { [key]: _drop, ...rest } = coding.dirs
  void _drop
  return { ...coding, dirs: rest }
}

/** Drop dir/file/req caches for `path` and every descendant under the same root. */
export function dropPathAndDescendants(coding: CodingSlice, root: string, path: string): CodingSlice {
  const rootPrefix = root + ':'
  const keepKey = (k: string): boolean => {
    if (!k.startsWith(rootPrefix)) return true
    const rel = k.slice(rootPrefix.length)
    return !isPathOrDescendant(rel, path)
  }
  const dirs: Record<string, DirState> = {}
  for (const [k, v] of Object.entries(coding.dirs)) {
    if (keepKey(k)) dirs[k] = v
  }
  const files: Record<string, CodingFileState> = {}
  for (const [k, v] of Object.entries(coding.files)) {
    if (keepKey(k)) files[k] = v
  }
  const _readReq: Record<string, string> = {}
  for (const [k, v] of Object.entries(coding._readReq)) {
    if (keepKey(k)) _readReq[k] = v
  }
  const _treeReq: Record<string, string> = {}
  for (const [k, v] of Object.entries(coding._treeReq)) {
    if (keepKey(k)) _treeReq[k] = v
  }
  return { ...coding, dirs, files, _readReq, _treeReq }
}

/**
 * Remap dir/file/req caches from `oldPath` onto `newPath` (including descendants),
 * then invalidate both parents so the tree reloads.
 */
export function remapPathAndDescendants(
  coding: CodingSlice,
  root: string,
  oldPath: string,
  newPath: string,
): CodingSlice {
  const rootPrefix = root + ':'
  const remapRecord = <T,>(rec: Record<string, T>): Record<string, T> => {
    const out: Record<string, T> = {}
    for (const [k, v] of Object.entries(rec)) {
      if (!k.startsWith(rootPrefix)) {
        out[k] = v
        continue
      }
      const rel = k.slice(rootPrefix.length)
      const mapped = remapPath(rel, oldPath, newPath)
      if (mapped == null) {
        out[k] = v
        continue
      }
      out[fileKey(root, mapped)] = v
    }
    return out
  }

  let next: CodingSlice = {
    ...coding,
    dirs: remapRecord(coding.dirs),
    files: remapRecord(coding.files),
    _readReq: remapRecord(coding._readReq),
    _treeReq: remapRecord(coding._treeReq),
  }
  next = invalidateDir(next, root, oldPath)
  next = invalidateDir(next, root, newPath)
  return next
}

export function reduceFileCreate(coding: CodingSlice, env: FileCreatePush): CodingSlice {
  if (env.error) return coding
  return invalidateDir(coding, env.root, env.path)
}

export function reduceFileRename(coding: CodingSlice, env: FileRenamePush): CodingSlice {
  if (env.error) return coding
  return remapPathAndDescendants(coding, env.root, env.oldPath, env.newPath)
}

export function reduceFileDelete(coding: CodingSlice, env: FileDeletePush): CodingSlice {
  if (env.error) return coding
  let next = dropPathAndDescendants(coding, env.root, env.path)
  next = invalidateDir(next, env.root, env.path)
  return next
}

export function reduceFileWriteBytes(coding: CodingSlice, env: FileWriteBytesPush): CodingSlice {
  if (env.error) return coding
  return invalidateDir(coding, env.root, env.path)
}
