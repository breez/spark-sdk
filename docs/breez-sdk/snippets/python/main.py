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
    sdk = await init_sdk()
    fetch_balance(sdk)
    listener_id = add_event_listener(sdk, SdkListener)

    # disconnect
    remove_event_listener(sdk, listener_id)
    await disconnect(sdk)


asyncio.run(main())
