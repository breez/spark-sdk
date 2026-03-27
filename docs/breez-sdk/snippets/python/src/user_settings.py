import logging
from breez_sdk_spark import (
    BreezSdk,
    StableBalanceActiveLabel,
    UpdateUserSettingsRequest,
)


async def get_user_settings(sdk: BreezSdk):
    # ANCHOR: get-user-settings
    try:
        user_settings = await sdk.get_user_settings()

        print(f"User settings: {user_settings}")
    except Exception as error:
        logging.error(error)
        raise
    # ANCHOR_END: get-user-settings

async def update_user_settings(sdk: BreezSdk):
    # ANCHOR: update-user-settings
    try:
        spark_private_mode_enabled = True
        await sdk.update_user_settings(
            request=UpdateUserSettingsRequest(
                spark_private_mode_enabled=spark_private_mode_enabled
            )
        )
    except Exception as error:
        logging.error(error)
        raise
    # ANCHOR_END: update-user-settings

async def activate_stable_balance(sdk: BreezSdk):
    # ANCHOR: activate-stable-balance
    try:
        await sdk.update_user_settings(
            request=UpdateUserSettingsRequest(
                spark_private_mode_enabled=None,
                stable_balance_active_label=StableBalanceActiveLabel.SET(label="USDB")
            )
        )
    except Exception as error:
        logging.error(error)
        raise
    # ANCHOR_END: activate-stable-balance

async def deactivate_stable_balance(sdk: BreezSdk):
    # ANCHOR: deactivate-stable-balance
    try:
        await sdk.update_user_settings(
            request=UpdateUserSettingsRequest(
                spark_private_mode_enabled=None,
                stable_balance_active_label=StableBalanceActiveLabel.UNSET()
            )
        )
    except Exception as error:
        logging.error(error)
        raise
    # ANCHOR_END: deactivate-stable-balance
