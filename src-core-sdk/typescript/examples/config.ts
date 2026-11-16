// Compile in a consumer project with NodeNext module resolution. No inference; reads and edits ~/.koma/config.json.
// Usage: node config.js [provider_uuid]  (without a uuid it only prints the view)
import { Client, SdkError } from '@koma/sdk';
const client = new Client({ executable: process.env.KOMA_EXECUTABLE ?? 'koma' });
const host = client.host();
try {
  if (!(await host.supports('config.get'))) throw new Error('this koma binary predates config.get; upgrade it');
  const view = await host.getConfig();
  console.log(`config.json ${view.file}, revision ${view.revision}`);
  for (const p of view.providers) console.log(`provider ${p.provider_uuid} ${p.name} ${p.endpoint} key=${p.key_state}${p.managed ? ' (extension)' : ''}`);
  for (const r of view.roles) console.log(`role ${r.role}: ${r.effective_model ?? '-'} (${r.origin})`);
  if (view.main_fallback) console.log(`main falls back: ${view.main_fallback}`);

  const target = process.argv[2];
  if (target) {
    try {
      // Preview first: a dry run runs every check and writes nothing.
      const preview = await host.deleteProvider({ provider_uuid: target, if_revision: view.revision, dry_run: true });
      console.log(`would remove models ${preview.removed.models_removed.join(', ') || '(none)'}; main reset: ${preview.removed.main_reset}`);
      // The same revision guards the real delete: a change by anyone else in between makes it fail with "stale".
      const done = await host.deleteProvider({ provider_uuid: target, if_revision: view.revision });
      console.log(`deleted; new revision ${done.config.revision}; propagation ${JSON.stringify(done.propagation)}`);
    } catch (e) {
      if (e instanceof SdkError && e.reason === 'stale') console.log('config.json changed since it was read; read it again and retry');
      else if (e instanceof SdkError && e.reason === 'managed') console.log('this provider belongs to an extension');
      else if (e instanceof SdkError && e.reason === 'not_found') console.log('no such provider (already deleted?)');
      else throw e;
    }
  }
} finally { await host.close(); await client.close(); }
