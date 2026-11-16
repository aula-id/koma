"""Show what the last N days cost, by model and by day.

    python usage.py          # the last 7 days
    python usage.py 30       # the last 30 days

Reads the usage ledger only. A dashboard should not poll this more often than every 30 seconds.
KOMA_EXECUTABLE overrides the installed binary.
"""
import asyncio
import os
import sys
import time
from koma_sdk import Client, LedgerState

async def main():
    days = int(sys.argv[1]) if len(sys.argv) > 1 else 7
    now = int(time.time())
    async with Client(os.environ.get('KOMA_EXECUTABLE', 'koma')) as client:
        async with client.host() as host:
            by_model = await host.usage_query(from_at=now - days * 86400, to_at=now, group_by='model')
            if by_model.ledger is LedgerState.MISSING:
                print('no usage has been recorded yet')
                return
            t = by_model.totals
            print(f'{t.calls} calls, {t.tokens_in} tokens in ({t.tokens_cached} cached), {t.tokens_out} out, ${t.cost:.4f}')
            for g in by_model.groups:
                print(f'  {g.key:32} {g.calls:5} calls  ${g.cost:.4f}')
            if by_model.groups_truncated:
                print('  (more models not shown)')
            tz = -time.timezone // 60
            for b in (await host.usage_query(from_at=now - days * 86400, to_at=now, bucket='day',
                                             tz_offset_minutes=tz)).buckets:
                print(time.strftime('%Y-%m-%d', time.localtime(b.start_at)), f'${b.cost:.4f}', b.calls, 'calls')

asyncio.run(main())
