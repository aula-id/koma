import io
from pathlib import Path
import sys
import unittest
import zipfile

from koma_sdk import (Client, IncompatibleAdapter, PropagationStatus, RemoteError, SkillDetail, SkillPage, SkillScope,
                      inline_zip)

FAKE = Path(__file__).resolve().parents[2] / 'conformance' / 'fake_adapter.py'
WORK = '/fake/workspace'


def package(name='release-notes', description='Writes release notes', manifest='SKILL.md'):
    out = io.BytesIO()
    with zipfile.ZipFile(out, 'w') as z:
        z.writestr(f'{name}/{manifest}', f'---\nname: {name}\ndescription: {description}\n---\nSummarise the commits.\n')
        z.writestr(f'{name}/notes.md', 'a companion\n')
    return out.getvalue()


class SkillsConformance(unittest.IsolatedAsyncioTestCase):
    def client(self, scenario='skills'):
        c = Client(sys.executable, adapter_args=(str(FAKE), '--scenario', scenario), close_timeout=.1)
        self.addAsyncCleanup(c.close)
        return c

    async def test_list_shows_shadowed_rows_and_diagnostics(self):
        async with self.client().host() as host:
            page = await host.skills_list(workdir=WORK, include_diagnostics=True)
            self.assertIsInstance(page, SkillPage)
            self.assertEqual(page.project_workdir, WORK)
            review = [s for s in page.skills if s.name == 'code-review']
            self.assertEqual({s.tier for s in review}, {'global', 'project-agents'})
            winner = next(s for s in review if s.effective)
            shadowed = next(s for s in review if not s.effective)
            self.assertEqual((winner.tier, shadowed.shadowed_by), ('project-agents', 'project-agents'))
            self.assertIs(winner.scope, SkillScope.PROJECT)
            self.assertTrue(page.diagnostics and page.diagnostics[0].path)
            tiers = {s.tier for s in page.skills}
            self.assertTrue({'claude', 'project-agent', 'extra-root:0'} <= tiers)
            self.assertFalse(next(s for s in page.skills if s.tier == 'claude').editable)
            computer = await host.skills_list(scope='computer')
            self.assertTrue(computer.skills and all(s.scope is SkillScope.COMPUTER for s in computer.skills))
            first = await host.skills_list(workdir=WORK, limit=2)
            self.assertEqual(len(first.skills), 2)
            self.assertIsNotNone(first.next_cursor)

    async def test_project_tiers_need_a_workdir_when_unbound(self):
        async with self.client().host() as host:
            with self.assertRaises(RemoteError) as ctx:
                await host.skills_list(scope='project')
            self.assertEqual((ctx.exception.code, ctx.exception.reason), ('invalid_request', 'workdir_required'))

    async def test_get_and_read_file(self):
        async with self.client().host() as host:
            detail = await host.skills_get('code-review', tier='global', include_raw=True)
            self.assertIsInstance(detail, SkillDetail)
            self.assertTrue(detail.raw)
            self.assertIn('notes/checklist.md', detail.companion_files)
            self.assertIsNone((await host.skills_get('code-review', tier='global')).raw)
            effective = await host.skills_get('code-review', workdir=WORK)
            self.assertEqual(effective.tier, 'project-agents')
            text = await host.skills_read_file('code-review', 'notes/checklist.md', tier='global')
            self.assertEqual(text.bytes, len(text.text.encode()))
            with self.assertRaises(RemoteError) as binary:
                await host.skills_read_file('code-review', 'bin/tool.bin', tier='global')
            self.assertEqual(binary.exception.reason, 'binary')
            with self.assertRaises(RemoteError) as missing:
                await host.skills_get('nothing-here', tier='global')
            self.assertEqual(missing.exception.code, 'not_found')

    async def test_create_update_remove_cycle(self):
        async with self.client().host() as host:
            made = await host.skills_create(scope='computer', name='triage', description='Triage issues',
                                            instruction='Label them.', triggers=['issue'], allowed_tools=['bash'])
            self.assertEqual((made.skill.name, made.skill.tier, made.skill.triggers), ('triage', 'global', ('issue',)))
            self.assertIsNone(made.skill.raw)
            self.assertIs(made.propagation.skills, PropagationStatus.NEXT_REBUILD)
            with self.assertRaises(RemoteError) as exists:
                await host.skills_create(scope='computer', name='triage', description='d', instruction='i')
            self.assertEqual(exists.exception.reason, 'exists')
            updated = await host.skills_update(tier='global', name='triage', generation=made.skill.generation,
                                               patch={'name': 'triage-bot', 'instruction': 'Label and assign.'})
            self.assertTrue(updated.renamed)
            self.assertEqual(updated.skill.instruction, 'Label and assign.')
            with self.assertRaises(RemoteError) as stale:
                await host.skills_update(tier='global', name='triage-bot', generation=made.skill.generation,
                                         patch={'description': 'x'})
            self.assertEqual((stale.exception.code, stale.exception.reason), ('invalid_state', 'stale'))
            gone = await host.skills_remove(tier='global', name='triage-bot', generation=updated.skill.generation)
            self.assertTrue(gone.removed)
            self.assertIsNone(gone.now_effective)
            with self.assertRaises(RemoteError) as again:
                await host.skills_remove(tier='global', name='triage-bot')
            self.assertEqual(again.exception.code, 'not_found')

    async def test_write_refusals(self):
        async with self.client().host() as host:
            with self.assertRaises(RemoteError) as ro:
                await host.skills_update(tier='claude', name='claude-helper', workdir=WORK, patch={'description': 'x'})
            self.assertEqual(ro.exception.reason, 'read_only')
            with self.assertRaises(RemoteError) as yaml:
                await host.skills_update(tier='global', name='odd-yaml', patch={'description': 'x'})
            self.assertEqual(yaml.exception.reason, 'unsafe_yaml')
            removed = await host.skills_remove(tier='project-agents', name='code-review', workdir=WORK)
            self.assertEqual(removed.now_effective.tier, 'global')

    async def test_install_from_bytes(self):
        async with self.client().host() as host:
            result = await host.skills_install(scope='computer', data=package())
            self.assertEqual((result.skill.name, result.skill.tier), ('release-notes', 'global'))
            self.assertIn('notes.md', result.skill.companion_files)
            with self.assertRaises(RemoteError) as exists:
                await host.skills_install(scope='computer', data=package())
            self.assertEqual(exists.exception.reason, 'exists')
            renamed = await host.skills_install(scope='computer', data=package(), name='notes-two')
            self.assertEqual(renamed.skill.name, 'notes-two')
            with self.assertRaises(RemoteError) as bad:
                await host.skills_install(scope='computer', data=b'not a zip')
            self.assertEqual(bad.exception.reason, 'invalid_package')
            with self.assertRaises(RemoteError) as lower:
                await host.skills_install(scope='computer', data=package('lower-case', manifest='skill.md'))
            self.assertEqual(lower.exception.reason, 'invalid_package')

    async def test_propagation_values_are_open(self):
        from koma_sdk.types import Propagation
        self.assertIs(Propagation.from_wire({'skills': 'room_restart'}).skills, PropagationStatus.ROOM_RESTART)
        self.assertIs(Propagation.from_wire({'skills': 'someday'}).skills, PropagationStatus.OTHER)
        self.assertIs(Propagation.from_wire({'room': 'not_supported'}).room, PropagationStatus.NOT_SUPPORTED)

    async def test_arguments_are_checked_before_sending(self):
        async with self.client().host() as host:
            calls = (host.skills_list(scope='all-of-them'), host.skills_list(workdir='rel'), host.skills_list(limit=1001),
                     host.skills_create(scope='user', name='x', description='d', instruction='i'),
                     host.skills_create(scope='computer', name='Bad Name', description='d', instruction='i'),
                     host.skills_create(scope='computer', name='readme', description='d', instruction='i'),
                     host.skills_create(scope='computer', name='ok', description='', instruction='i'),
                     host.skills_create(scope='computer', name='ok', description='d', instruction='i',
                                        triggers=['t'] * 33),
                     host.skills_update(tier='global', name='x', patch={}),
                     host.skills_update(tier='global', name='x', patch={'color': 'red'}),
                     host.skills_update(tier='', name='x', patch={'description': 'd'}),
                     host.skills_install(scope='computer'),
                     host.skills_install(scope='computer', data=b'x', path='/tmp/x.zip'),
                     host.skills_install(scope='computer', path='relative.zip'),
                     host.skills_remove(tier='', name='x'))
            for call in calls:
                with self.assertRaises(ValueError):
                    await call
            self.assertIsNone(host.hello)

    async def test_inline_zip_checks_the_cap_before_encoding(self):
        self.assertEqual(inline_zip(b'PK'), 'UEs=')
        with self.assertRaises(ValueError):
            inline_zip(b'x' * 11, max_bytes=10)
        with self.assertRaises(ValueError):
            inline_zip('not bytes')
        async with self.client().host() as host:
            await host.skills_list(scope='computer')
            cap = host.hello.limits['skills']['inline_zip_bytes']
            self.assertEqual(cap, 29360128)
            host.hello.limits['skills']['inline_zip_bytes'] = 8
            with self.assertRaises(ValueError):
                await host.skills_install(scope='computer', data=b'123456789')

    async def test_legacy_adapter_is_refused(self):
        async with self.client('legacy_hello').host() as host:
            with self.assertRaises(IncompatibleAdapter):
                await host.skills_list()
