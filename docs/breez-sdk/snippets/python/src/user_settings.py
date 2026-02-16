import logging
from breez_sdk_spark import (
    BreezClient,
    UpdateUserSettingsRequest,
)


async def get_user_settings(client: BreezClient):
    # ANCHOR: get-user-settings
    try:
        user_settings = await client.get_user_settings()

        print(f"User settings: {user_settings}")
    except Exception as error:
        logging.error(error)
        raise
    # ANCHOR_END: get-user-settings

async def update_user_settings(client: BreezClient):
    # ANCHOR: update-user-settings
    try:
        spark_private_mode_enabled = True
        await client.update_user_settings(
            request=UpdateUserSettingsRequest(
                spark_private_mode_enabled=spark_private_mode_enabled
            )
        )
    except Exception as error:
        logging.error(error)
        raise
    # ANCHOR_END: update-user-settings
