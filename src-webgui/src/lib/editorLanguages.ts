// ext -> Monarch language id (all registered above). JSON has no basic-languages
// tokenizer (its highlighter ships with the worker-backed json service we omit),
// so it borrows the javascript tokenizer — close enough for coloured JSON diffs.
// Unknown extensions fall back to plaintext.
const EXT_LANG: Record<string, string> = {
  ts: 'typescript', tsx: 'typescript', mts: 'typescript', cts: 'typescript',
  js: 'javascript', jsx: 'javascript', mjs: 'javascript', cjs: 'javascript',
  json: 'javascript', jsonc: 'javascript',
  css: 'css', scss: 'scss', sass: 'scss', less: 'less',
  html: 'html', htm: 'html', xhtml: 'html', xml: 'xml', svg: 'xml',
  md: 'markdown', markdown: 'markdown', mdx: 'markdown',
  rs: 'rust',
  py: 'python', pyi: 'python',
  go: 'go',
  sh: 'shell', bash: 'shell', zsh: 'shell',
  yaml: 'yaml', yml: 'yaml',
  sql: 'sql',
  c: 'cpp', h: 'cpp', hpp: 'cpp', hh: 'cpp', cpp: 'cpp', cc: 'cpp', cxx: 'cpp',
  java: 'java',
  php: 'php', phtml: 'php', php3: 'php', php4: 'php', php5: 'php', phps: 'php',
  ini: 'ini', toml: 'ini', cfg: 'ini', conf: 'ini',
}

export function langFromPath(path: string): string {
  const file = path.split('/').pop() ?? path
  const dot = file.lastIndexOf('.')
  const ext = dot > 0 ? file.slice(dot + 1).toLowerCase() : ''
  return EXT_LANG[ext] ?? 'plaintext'
}
