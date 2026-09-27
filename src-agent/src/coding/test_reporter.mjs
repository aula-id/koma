// Node's documented custom test-reporter interface, loaded only on explicit Run.
export default async function* (events) {
  for await (const { type, data } of events) {
    if (type === 'test:stdout' || type === 'test:stderr') {
      yield data.message;
    } else if (['test:start', 'test:pass', 'test:fail'].includes(type)) {
      yield '\nKOMA_TEST ' + JSON.stringify({
        id: `${data.file || ''}:${data.line || 0}:${data.column || 0}:${data.name}`,
        label: data.name,
        nesting: data.nesting,
        file: data.file,
        line: data.line,
        status: type === 'test:start' ? 'running' : data.skip || data.todo ? 'skipped' : type === 'test:pass' ? 'passed' : 'failed',
        durationMs: data.details?.duration_ms,
        message: String(data.details?.error?.stack || data.details?.error?.message || '').slice(0, 4096)
      }) + '\n';
    }
  }
}
