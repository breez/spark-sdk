from breez_sdk_spark import BreezClient

async def list_fiat_currencies(client: BreezClient):
    # ANCHOR: list-fiat-currencies
    try:
        response = await client.fiat().currencies()
    except Exception as error:
        print(error)
        raise
    # ANCHOR_END: list-fiat-currencies

async def list_fiat_rates(client: BreezClient):
    # ANCHOR: list-fiat-rates
    try:
        response = await client.fiat().rates()
    except Exception as error:
        print(error)
        raise
    # ANCHOR_END: list-fiat-rates
