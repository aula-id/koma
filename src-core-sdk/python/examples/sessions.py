"""List sessions, newest first, and optionally rename or stop one.

    python sessions.py                       # every session, with its kind and whether it is live
    python sessions.py --rooms               # rooms only
    python sessions.py SESSION_ID NEW NAME   # rename a room, or a session that is not live
    python sessions.py --stop SESSION_ID     # stop that session's daemon (disconnects every client of it)

No inference runs. KOMA_EXECUTABLE overrides the installed binary.
"""
import asyncio
import os
import sys
from koma_sdk import Client, RemoteError

async def main():
    args = sys.argv[1:]
    async with Client(os.environ.get('KOMA_EXECUTABLE', 'koma')) as client:
        async with client.host() as host:
            try:
                if args[:1] == ['--stop']:
                    result = await host.sessions_stop(args[1])
                    print('stopped' if result.was_live else 'was not running')
                elif len(args) >= 2 and not args[0].startswith('--'):
                    row = (await host.sessions_rename(args[0], ' '.join(args[1:]))).session
                    print('renamed to', row.name)
                else:
                    cursor = None
                    while True:
                        page = await host.sessions_list(kind='room' if args[:1] == ['--rooms'] else 'all',
                                                        limit=100, cursor=cursor)
                        for s in page.sessions:
                            state = 'live' if s.live else 'idle'
                            print(f'{s.session_id}  {s.kind.value:6} {state:5} {s.name or "-"}  {s.workdir or ""}')
                        cursor = page.next_cursor
                        if cursor is None:
                            break
            except RemoteError as exc:
                # `live`: another adapter holds the session; `session_locked`, `bound_self`: see exc.reason.
                print(f'refused: {exc.reason or exc.code}: {exc.message}')

asyncio.run(main())
