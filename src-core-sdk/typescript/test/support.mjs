import { fileURLToPath } from 'node:url';
import { crc32 } from 'node:zlib';
import { Client } from '../dist/index.js';

export const fake = fileURLToPath(new URL('../../conformance/fake_adapter.py', import.meta.url));
export const python = process.env.PYTHON ?? 'python3';
export function client(scenario = 'normal', extra = {}) {
  return new Client({ executable: python, adapterArgs: [fake, '--scenario', scenario], timeoutMs: 3000, ...extra });
}
/** Reads room events until `count` arrived, the iterator ended, or `ms` passed. */
export async function collect(room, count, ms = 1500) {
  const out = [];
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), ms);
  try { for await (const event of room.events({ signal: controller.signal })) { out.push(event); if (out.length >= count) break; } }
  catch (e) { if (e.code !== 'cancelled') throw e; }
  finally { clearTimeout(timer); }
  return out;
}
const u16 = n => { const b = Buffer.alloc(2); b.writeUInt16LE(n); return b; };
const u32 = n => { const b = Buffer.alloc(4); b.writeUInt32LE(n); return b; };
/** A zip of stored (uncompressed) entries: { 'dir/SKILL.md': 'text' }. */
export function zip(files) {
  const local = [], central = [];
  let offset = 0;
  for (const [name, text] of Object.entries(files)) {
    const n = Buffer.from(name), d = Buffer.from(text), c = crc32(d);
    const head = Buffer.concat([u32(0x04034b50), u16(20), u16(0), u16(0), u16(0), u16(0x21), u32(c), u32(d.length), u32(d.length), u16(n.length), u16(0), n]);
    local.push(head, d);
    central.push(Buffer.concat([u32(0x02014b50), u16(20), u16(20), u16(0), u16(0), u16(0), u16(0x21), u32(c), u32(d.length), u32(d.length), u16(n.length), u16(0), u16(0), u16(0), u16(0), u32(0), u32(offset), n]));
    offset += head.length + d.length;
  }
  const dir = Buffer.concat(central);
  const end = Buffer.concat([u32(0x06054b50), u16(0), u16(0), u16(central.length), u16(central.length), u32(dir.length), u32(offset), u16(0)]);
  return new Uint8Array(Buffer.concat([...local, dir, end]));
}
