"""Read the global config, then delete a provider guarded by a dry run and its revision.

    python config.py                      # list providers, models and role routes
    python config.py PROVIDER_UUID        # dry run only: show what the delete would remove
    python config.py PROVIDER_UUID --yes  # delete, refused if config.json changed since the dry run

No session or daemon is started and no inference runs. KOMA_EXECUTABLE overrides the installed binary.
"""
import asyncio
import os
import sys
from koma_sdk import Client, RemoteError

async def main():
    args = [a for a in sys.argv[1:] if a != '--yes']
    async with Client(os.environ.get('KOMA_EXECUTABLE', 'koma')) as client:
        async with client.host() as host:
            view = await host.config_get()
            print(f'config.json: {view.file.value}, revision {view.revision}')
            for p in view.providers:
                print(f'  provider {p.provider_uuid} {p.name!r} {p.endpoint} key={p.key_state.value}'
                      f' models={p.model_count}{" (managed)" if p.managed else ""}')
            for m in view.models:
                roles = ','.join(r.value for r in m.roles) or '-'
                print(f'  model {m.model_uuid} {m.model_id} via {m.provider_name or m.provider_kind.value} roles={roles}')
            for r in view.roles:
                print(f'  {r.role.value}: {r.effective_model or "-"} ({r.origin.value})')
            if view.main_fallback is not None:
                print('Main falls back:', view.main_fallback.value)
            if not args:
                return
            try:
                plan = await host.config_providers_delete(args[0], if_revision=view.revision, dry_run=True)
            except RemoteError as exc:
                print(f'refused: {exc.reason or exc.code}: {exc.message}')
                return
            print('would remove models:', ', '.join(plan.removed.models_removed) or 'none',
                  '(Main resets)' if plan.removed.main_reset else '')
            if '--yes' not in sys.argv[1:]:
                print('dry run only; pass --yes to delete')
                return
            try:
                done = await host.config_providers_delete(args[0], if_revision=plan.config.revision)
            except RemoteError as exc:
                # `stale`: config.json changed after the dry run; read it again before deciding.
                print(f'refused: {exc.reason or exc.code}: {exc.message}')
                return
            print('deleted; new revision', done.config.revision)
            print('propagation:', {k: v.value for k, v in vars(done.propagation).items() if v is not None})

asyncio.run(main())
