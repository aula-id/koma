// Node emits test:start/pass/fail in declaration order, including nested suites.
// Retain the full ancestor name for --test-name-pattern on a later run.
export default async function* (events) {
  const stacks = new Map();
  const names = new Map();
  for await (const { type, data } of events) {
    if (type === 'test:stdout' || type === 'test:stderr') {
      yield data.message;
    } else if (['test:start', 'test:pass', 'test:fail'].includes(type)) {
      const entry = data.entryFile || data.file || '';
      const id = `${entry}:${data.file || ''}:${data.line || 0}:${data.column || 0}:${data.name}`;
      const nesting = Math.max(0, Math.min(256, data.nesting || 0));
      if (type === 'test:start') {
        const stack = (stacks.get(entry) || []).slice(0, nesting);
        stack[nesting] = data.name;
        if (stacks.size < 5000 || stacks.has(entry)) stacks.set(entry, stack);
        if (names.size < 5000 || names.has(id)) names.set(id, stack.join(' '));
      }
      yield '\nKOMA_TEST ' + JSON.stringify({
        id, label: data.name, nesting, file: data.file, entryFile: entry,
        selector: names.get(id), line: data.line,
        status: type === 'test:start' ? 'running' : data.skip || data.todo ? 'skipped' : type === 'test:pass' ? 'passed' : 'failed',
        durationMs: data.details?.duration_ms,
        message: String(data.details?.error?.stack || data.details?.error?.message || '').slice(0, 4096)
      }) + '\n';
    }
  }
}
