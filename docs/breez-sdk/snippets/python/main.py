#!/usr/bin/env python
import asyncio
from src.getting_started import (
    disconnect,
    init_sdk,
    set_logger,
    fetch_balance,
    add_event_listener,
    remove_event_listener,
    SdkLogger,
    SdkListener,
)


async def main():
    # getting started
    set_logger(SdkLogger)
    client = await init_sdk()
    fetch_balance(client)
    listener_id = add_event_listener(client, SdkListener)

    # disconnect
    remove_event_listener(client, listener_id)
    await disconnect(client)


asyncio.run(main())
