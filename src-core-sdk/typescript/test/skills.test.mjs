import assert from 'node:assert/strict';
import { test } from 'node:test';
import { SdkError } from '../dist/index.js';
import { client, zip } from './support.mjs';

const SKILL = name => `---\nname: ${name}\ndescription: A zipped skill\n---\nDo the thing.\n`;

test('skills.list, get and read_file address skills by tier and name', async () => {
  const c = client('skills'), host = c.host();
  try {
    const global = await host.listSkills({ scope: 'computer' });
    assert.deepEqual(global.skills.map(s => s.name).sort(), ['code-review', 'deploy-notes', 'odd-yaml']);
    assert.equal(global.project_workdir, null);
    await assert.rejects(host.listSkills({ scope: 'project' }), e => e instanceof SdkError && e.reason === 'workdir_required');
    const all = await host.listSkills({ workdir: '/fake/workspace', include_diagnostics: true });
    assert.equal(all.project_workdir, '/fake/workspace'); assert.equal(all.diagnostics.length, 1);
    const shadowed = all.skills.filter(s => s.name === 'code-review');
    assert.equal(shadowed.length, 2); assert.equal(shadowed.filter(s => s.effective).length, 1);
    assert.ok(shadowed.some(s => s.shadowed_by === 'project-agents'));
    const pages = []; let cursor;
    do { const p = await host.listSkills({ workdir: '/fake/workspace', limit: 2, cursor }); pages.push(...p.skills); cursor = p.next_cursor ?? undefined; } while (cursor);
    assert.equal(pages.length, all.skills.length);
    const detail = await host.getSkill({ tier: 'global', name: 'code-review', include_raw: true });
    assert.ok(detail.instruction.length > 0 && detail.raw.length > 0 && detail.companion_files.includes('notes/checklist.md'));
    assert.equal((await host.getSkill({ tier: 'global', name: 'code-review' })).raw, null);
    const file = await host.readSkillFile({ tier: 'global', name: 'code-review', path: 'notes/checklist.md' });
    assert.equal(file.bytes, Buffer.byteLength(file.text));
    await assert.rejects(host.readSkillFile({ tier: 'global', name: 'code-review', path: 'bin/tool.bin' }), e => e.reason === 'binary');
    await assert.rejects(host.getSkill({ tier: 'global', name: 'missing' }), e => e.code === 'not_found');
  } finally { await host.close(); await c.close(); }
});

test('skills create, update, remove with generation and read-only tiers', async () => {
  const c = client('skills'), host = c.host();
  try {
    const made = await host.createSkill({ scope: 'computer', name: 'fresh-one', description: 'Fresh', instruction: 'Steps.', triggers: ['go'] });
    assert.equal(made.skill.tier, 'global'); assert.equal(made.skill.raw, null); assert.equal(made.propagation.skills, 'next_rebuild');
    await assert.rejects(host.createSkill({ scope: 'computer', name: 'fresh-one', description: 'Fresh', instruction: 'Steps.' }), e => e.reason === 'exists');
    await assert.rejects(host.createSkill({ scope: 'computer', name: 'Bad Name', description: 'x', instruction: 'y' }), e => e.reason === 'invalid_name');
    await assert.rejects(host.updateSkill({ tier: 'global', name: 'fresh-one', patch: { name: 'Bad' } }), e => e.code === 'invalid_input' && e.reason === 'invalid_name');
    const updated = await host.updateSkill({ tier: 'global', name: 'fresh-one', generation: made.skill.generation, patch: { description: 'Changed' } });
    assert.equal(updated.skill.description, 'Changed'); assert.equal(updated.renamed, false);
    assert.notEqual(updated.skill.generation, made.skill.generation);
    await assert.rejects(host.updateSkill({ tier: 'global', name: 'fresh-one', generation: made.skill.generation, patch: { description: 'x' } }), e => e.reason === 'stale');
    const renamed = await host.updateSkill({ tier: 'global', name: 'fresh-one', patch: { name: 'fresher' } });
    assert.equal(renamed.renamed, true);
    await assert.rejects(host.updateSkill({ tier: 'global', name: 'odd-yaml', patch: { description: 'x' } }), e => e.reason === 'unsafe_yaml');
    await assert.rejects(host.updateSkill({ tier: 'claude', workdir: '/fake/workspace', name: 'claude-helper', patch: { description: 'x' } }), e => e.reason === 'read_only');
    const removed = await host.removeSkill({ tier: 'global', name: 'fresher' });
    assert.equal(removed.removed, true); assert.equal(removed.now_effective, null);
    await assert.rejects(host.removeSkill({ tier: 'global', name: 'fresher' }), e => e.code === 'not_found');
  } finally { await host.close(); await c.close(); }
});

test('skills.install from zip bytes, base64 and the size gate', async () => {
  const c = client('skills'), host = c.host();
  try {
    const bytes = zip({ 'zipped/SKILL.md': SKILL('zipped') });
    const done = await host.installSkillZip(bytes, { scope: 'computer' });
    assert.equal(done.skill.name, 'zipped'); assert.equal(done.skill.tier, 'global');
    await assert.rejects(host.installSkillZip(bytes, { scope: 'computer' }), e => e.reason === 'exists');
    const named = await host.installSkill({ scope: 'computer', name: 'other-name', data_b64: Buffer.from(zip({ 'x/SKILL.md': SKILL('other-name') })).toString('base64') });
    assert.equal(named.skill.name, 'other-name');
    await assert.rejects(host.installSkill({ scope: 'computer' }), e => e instanceof SdkError && e.code === 'invalid_input' && e.reason === 'conflicting_params');
    await assert.rejects(host.installSkill({ scope: 'computer', path: '/a', data_b64: 'AA==' }), e => e.reason === 'conflicting_params');
    await assert.rejects(host.installSkillZip(new Uint8Array(0), { scope: 'computer' }), e => e.code === 'invalid_input');
    // The cap is checked before encoding and before anything is written to the adapter.
    const before = (await c.probe()).limits.skills.inline_zip_bytes;
    assert.equal(before, 29360128);
    await assert.rejects(host.installSkillZip(new Uint8Array(before + 1), { scope: 'computer' }),
      e => e instanceof SdkError && e.reason === 'too_large' && e.delivery === 'not_sent' && e.requestId === undefined);
    await assert.rejects(host.installSkill({ scope: 'computer', data_b64: 'A'.repeat(39146841) }), e => e.reason === 'too_large' && e.requestId === undefined);
    await assert.rejects(host.installSkillZip(zip({ 'bad/skill.md': SKILL('bad') }), { scope: 'computer' }), e => e.code === 'invalid_request');
  } finally { await host.close(); await c.close(); }
});

test('skill calls in a session use its workdir and report next_rebuild', async () => {
  const c = client('skills');
  try {
    const session = await c.create({ cwd: '/fake/workspace' });
    const project = await session.listSkills({ scope: 'project' });
    assert.ok(project.skills.length > 0 && project.skills.every(s => s.scope === 'project'));
    const made = await session.createSkill({ scope: 'project', name: 'in-session', description: 'd', instruction: 'i' });
    assert.equal(made.skill.tier, 'project-agents'); assert.equal(made.propagation.skills, 'next_rebuild');
  } finally { await c.close(); }
});
