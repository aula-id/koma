// Compile in a consumer project with NodeNext module resolution. Reads the usage ledger; no model is called.
// Usage: node usage.js [days]
import { Client } from '@koma/sdk';
const client = new Client({ executable: process.env.KOMA_EXECUTABLE ?? 'koma' });
const host = client.host();
try {
  const days = Number(process.argv[2] ?? 7);
  const to = Math.floor(Date.now() / 1000);
  const report = await host.queryUsage({ from_at: to - days * 86400, to_at: to, group_by: 'model', bucket: 'day' });
  if (report.ledger === 'missing') console.log('no usage recorded yet');
  console.log(`${report.totals.calls} calls, ${report.totals.tokens_in} in, ${report.totals.tokens_out} out, cost ${report.totals.cost.toFixed(4)}`);
  for (const g of report.groups ?? []) console.log(`  ${g.key}: ${g.calls} calls, cost ${g.cost.toFixed(4)}`);
  if (report.groups_truncated) console.log('  (more models than the limit)');
  for (const b of report.buckets ?? []) console.log(`${new Date(b.start_at * 1000).toISOString().slice(0, 10)} ${b.calls}`);
} finally { await host.close(); await client.close(); }
