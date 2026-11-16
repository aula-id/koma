// Compile in a consumer project with NodeNext module resolution. No inference; lists sessions and, with an id, renames it.
// Usage: node sessions.js [session_id new-name]
import { Client, SdkError } from '@koma/sdk';
const client = new Client({ executable: process.env.KOMA_EXECUTABLE ?? 'koma' });
const host = client.host();
try {
  let cursor: string | undefined;
  do {
    const page = await host.listSessions({ limit: 50, cursor });
    for (const s of page.sessions) console.log(`${s.session_id} ${s.kind} ${s.live ? 'live' : 'idle'} ${s.name ?? '(unnamed)'} ${s.workdir ?? ''}`);
    cursor = page.next_cursor ?? undefined;
  } while (cursor);

  const [id, name] = process.argv.slice(2);
  if (id && name) {
    try { console.log((await host.renameSession({ session_id: id, name })).session.name); }
    catch (e) {
      // Another live coding session is renamed from an adapter bound to it, or after it is stopped.
      if (e instanceof SdkError && e.reason === 'live') console.log('that session is running; stop it first');
      else throw e;
    }
  }
} finally { await host.close(); await client.close(); }
