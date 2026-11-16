// Compile in a consumer project with NodeNext module resolution. Opens the room of a coding session and prints its events.
// Attaching takes the room's live view: a terminal showing the same room is disconnected.
// Usage: node room.js <parent_session_id> ["message for the room"]
import { Client } from '@koma/sdk';
const client = new Client({ executable: process.env.KOMA_EXECUTABLE ?? 'koma' });
const [parent, message] = process.argv.slice(2);
if (!parent) throw new Error('usage: room.js <parent_session_id> [message]');
const room = await client.roomOpen({ parent_session_id: parent });
try {
  console.log(`room ${room.sessionId}, epoch ${room.epoch}, avatars ${room.attachResult.roster.join(', ')}`);
  for (const line of room.attachResult.tail) console.log(`  [${line.kind}] ${line.author}: ${line.text ?? ''}`);
  if (message) console.log(await room.send({ text: message }));

  const stop = new AbortController();
  setTimeout(() => stop.abort(), 30_000);
  try {
    for await (const event of room.events({ signal: stop.signal })) {
      if (event.event === 'room.line') console.log(`[${event.line.kind}] ${event.line.author}: ${event.line.text ?? ''}${event.recovered ? ' (recovered)' : ''}`);
      else if (event.event === 'room.notice') console.log(`notice ${event.code}: ${event.text}`);
      else if (event.event === 'room.closed') {
        // Never re-attach on your own: two viewers would take the room from each other forever.
        console.log(`view ended: ${event.reason}${event.reason === 'taken_over' ? ' (open in another client)' : ''}`);
      }
    }
  } catch (e) { if ((e as { code?: string }).code !== 'cancelled') throw e; }
  const status = await room.status();
  console.log(`open ${status.counts.open}, verified ${status.counts.verified}, reports ${status.counts.reports}`);
} finally { await room.close(); await client.close(); }
