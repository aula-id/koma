// Compile in a consumer project with NodeNext module resolution. Lists skills and installs a zip file.
// Usage: node skills.js [workdir] [skill.zip]
import { readFile } from 'node:fs/promises';
import { Client, SdkError } from '@koma/sdk';
const client = new Client({ executable: process.env.KOMA_EXECUTABLE ?? 'koma' });
const host = client.host();
try {
  const [workdir, zipPath] = process.argv.slice(2);
  const list = await host.listSkills({ workdir, include_diagnostics: true });
  for (const s of list.skills) console.log(`${s.effective ? '*' : ' '} ${s.tier}/${s.name}${s.editable ? '' : ' (read only)'}: ${s.description}`);
  for (const d of list.diagnostics) console.log(`skipped ${d.path}: ${d.error}`);

  if (zipPath) {
    try {
      // The size is checked before encoding; a larger zip needs installSkill({ path }) on the adapter host.
      const done = await host.installSkillZip(await readFile(zipPath), { scope: 'computer' });
      console.log(`installed ${done.skill.name}; running sessions pick it up: ${done.propagation.skills}`);
    } catch (e) {
      if (e instanceof SdkError && e.reason === 'exists') console.log('a skill with that name is already installed');
      else if (e instanceof SdkError && e.reason === 'too_large') console.log('zip too large to send inline');
      else throw e;
    }
  }
} finally { await host.close(); await client.close(); }
