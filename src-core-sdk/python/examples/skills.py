"""List skills, or install one from a zip file.

    python skills.py /absolute/workdir               # every tier, shadowed skills marked
    python skills.py /absolute/workdir pack.zip      # install pack.zip into the project (.agents/skills)
    python skills.py --computer pack.zip             # install it for every project (~/.koma/skills)

The zip holds one directory with a SKILL.md. Running sessions see a new skill at their next system-prompt
rebuild. KOMA_EXECUTABLE overrides the installed binary.
"""
import asyncio
import os
import sys
from koma_sdk import Client, RemoteError

async def main():
    where, *rest = sys.argv[1:] or ['--computer']
    workdir = None if where == '--computer' else where
    async with Client(os.environ.get('KOMA_EXECUTABLE', 'koma')) as client:
        async with client.host() as host:
            try:
                if rest:
                    data = open(rest[0], 'rb').read()   # inline up to 28 MiB; use path= for a larger zip
                    done = await host.skills_install(scope='computer' if workdir is None else 'project',
                                                     workdir=workdir, data=data)
                    print(f'installed {done.skill.name} in tier {done.skill.tier}; propagation {done.propagation.skills.value}')
                    return
                page = await host.skills_list(scope='all' if workdir else 'computer', workdir=workdir,
                                              include_diagnostics=True)
                for s in page.skills:
                    note = '' if s.effective else f'  (shadowed by {s.shadowed_by})'
                    print(f'{s.tier:16} {s.name:24} {"editable" if s.editable else "read only"}{note}')
                for d in page.diagnostics:
                    print('skipped', d.path, '-', d.error)
            except RemoteError as exc:
                # `exists`, `invalid_package`, `workdir_required`, `too_large`, ...
                print(f'refused: {exc.reason or exc.code}: {exc.message}')

asyncio.run(main())
