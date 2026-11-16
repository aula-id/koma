"""Open the room of a coding session, print what happens in it, and optionally say something.

    python room.py PARENT_SESSION_ID                 # watch, until Ctrl-C
    python room.py PARENT_SESSION_ID "scan /login"   # send a message, then watch

Opening takes the room's live view over from any other client, the terminal included. If another
client takes it back, this script reports it and stops; it never takes the room back by itself.
Needs a usable Main model for a room that is not running yet. KOMA_EXECUTABLE overrides the binary.
"""
import asyncio
import os
import sys
from koma_sdk import (Client, CloseReason, RemoteError, RoomClosedEvent, RoomDeltaEvent, RoomGapEvent,
                      RoomLineEvent, RoomNoticeEvent, RoomStatusEvent)

async def main():
    parent, *message = sys.argv[1:]
    async with Client(os.environ.get('KOMA_EXECUTABLE', 'koma')) as client:
        try:
            room = await client.room_open(parent, tail=20)
        except RemoteError as exc:
            print(f'refused: {exc.reason or exc.code}: {exc.message}')
            return
        async with room:
            print('room', room.session_id, 'epoch', room.epoch, 'team', ', '.join(room.attached.roster))
            for line in room.attached.tail:
                print(f'  [{line.author}] {line.text or line.kind.value}')
            if message:
                await room.send(' '.join(message))
            async for event in room.events():
                if isinstance(event, RoomLineEvent):
                    print(f'[{event.line.author}] {event.line.text or event.line.kind.value}')
                elif isinstance(event, RoomStatusEvent):
                    c = event.status.counts
                    print(f'-- {c.open} open, {c.verified} verified, {len(event.status.lanes)} lanes')
                elif isinstance(event, RoomNoticeEvent):
                    print('-- notice:', event.code.value, event.text)
                elif isinstance(event, RoomGapEvent):
                    print(f'-- {event.dropped_lines} events were dropped; the missed lines follow')
                elif isinstance(event, RoomClosedEvent):
                    if event.reason is CloseReason.TAKEN_OVER:
                        print('-- the room is open in another client; run again to take it back')
                    else:
                        print('-- the room closed:', event.reason.value)
                elif isinstance(event, RoomDeltaEvent):
                    pass   # streaming text; the complete line follows as a RoomLineEvent

try:
    asyncio.run(main())
except KeyboardInterrupt:
    pass
