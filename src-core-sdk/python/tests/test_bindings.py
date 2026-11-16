from pathlib import Path
import sys
import unittest

from koma_sdk import Client, DeliveryUnknown, RemoteError

FAKE = Path(__file__).resolve().parents[2] / 'conformance' / 'fake_adapter.py'


class WrongBinding(unittest.IsolatedAsyncioTestCase):
    def client(self, *extra):
        c = Client(sys.executable, adapter_args=(str(FAKE), '--scenario', 'wrong_binding', *extra), close_timeout=.1)
        self.addAsyncCleanup(c.close)
        return c

    async def raw(self, client, method, params=None, mutation=False):
        transport, _ = await client._connect(())
        try:
            return await transport.request(method, params or {}, mutation=mutation, timeout=5,
                                           envelope=lambda i, m, p: {'id': i, 'method': m, 'params': p})
        finally:
            await transport.close()

    async def test_unbound_errors_carry_binding_and_allowed(self):
        c = self.client()
        with self.assertRaises(RemoteError) as ctx:
            await self.raw(c, 'state')
        e = ctx.exception
        self.assertEqual((e.code, e.message), ('invalid_state', 'no session bound'))
        self.assertEqual((e.reason, e.binding, e.allowed), ('wrong_binding', 'unbound', ('coding',)))
        self.assertFalse(e.written)
        self.assertIsNone(e.detail)
        with self.assertRaises(RemoteError) as ctx:
            await self.raw(c, 'config.effort.get')
        e = ctx.exception
        self.assertEqual(e.message, 'config.effort.get requires binding coding; this adapter is unbound')
        self.assertEqual((e.reason, e.binding, e.allowed), ('wrong_binding', 'unbound', ('coding',)))
        with self.assertRaises(RemoteError) as ctx:
            await self.raw(c, 'room.status')
        self.assertEqual(ctx.exception.allowed, ('room_live', 'room_closed'))

    async def test_coding_errors(self):
        s = await self.client().create('/fake/work')
        with self.assertRaises(RemoteError) as ctx:
            await s._request('create', {'cwd': '/fake/other'}, mutation=True)
        e = ctx.exception
        self.assertEqual((e.code, e.message, e.reason), ('invalid_state', 'session already bound', 'already_bound'))
        self.assertEqual((e.binding, e.allowed), ('coding', ('unbound',)))
        # A not_sent refusal of a mutation stays a RemoteError; the session keeps working.
        self.assertEqual((await s.info()).name, 'fake session')
        # A session-scope role other than main is refused before the binding is checked.
        with self.assertRaises(RemoteError) as ctx:
            await s._request('config.roles.unassign', {'role': 'planner', 'scope': 'session',
                                                       'if_revision': 'x'}, mutation=True)
        self.assertNotIsInstance(ctx.exception, DeliveryUnknown)
        self.assertEqual((ctx.exception.code, ctx.exception.reason), ('invalid_request', 'out_of_range'))

    async def test_unbound_detach_keeps_its_v1_error(self):
        c = self.client()
        with self.assertRaises(RemoteError) as ctx:
            await self.raw(c, 'detach', mutation=True)
        e = ctx.exception
        self.assertEqual((e.code, e.message, e.reason, e.binding, e.allowed),
                         ('invalid_state', 'no session bound', None, None, ()))
        host = c.host()
        self.addAsyncCleanup(host.close)
        self.assertEqual((await host.config_get()).file.value, 'missing')

    async def test_room_closed_socket_write(self):
        c = self.client('--binding', 'room_closed')
        with self.assertRaises(RemoteError) as ctx:
            await self.raw(c, 'room.send', {'text': 'hi'}, mutation=True)
        e = ctx.exception
        self.assertNotIsInstance(e, DeliveryUnknown)
        self.assertEqual((e.code, e.reason, e.binding, e.allowed),
                         ('invalid_state', 'room_closed', 'room_closed', ('room_live',)))


if __name__ == '__main__':
    unittest.main()
