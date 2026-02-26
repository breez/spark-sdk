from breez_sdk_spark.breez_sdk_spark_bindings import *
from breez_sdk_spark.breez_sdk_spark import *

import asyncio
import functools
from breez_sdk_spark.breez_sdk_spark import (
    connect as _original_connect,
    connect_with_signer as _original_connect_with_signer,
    SdkBuilder as _OriginalSdkBuilder,
    uniffi_set_event_loop,
)


def _ensure_event_loop():
    uniffi_set_event_loop(asyncio.get_running_loop())


@functools.wraps(_original_connect)
async def connect(*args, **kwargs):
    _ensure_event_loop()
    return await _original_connect(*args, **kwargs)


@functools.wraps(_original_connect_with_signer)
async def connect_with_signer(*args, **kwargs):
    _ensure_event_loop()
    return await _original_connect_with_signer(*args, **kwargs)


class SdkBuilder(_OriginalSdkBuilder):
    async def build(self):
        _ensure_event_loop()
        return await super().build()
