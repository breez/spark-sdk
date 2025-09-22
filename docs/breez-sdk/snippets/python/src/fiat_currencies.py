from breez_sdk_spark import BreezSdk

async def list_fiat_currencies(sdk: BreezSdk):
   # ANCHOR: list-fiat-currencies
   try:
      response = await sdk.list_fiat_currencies()
   except Exception as error:
      print(error)
      raise
   # ANCHOR_END: list-fiat-currencies

async def list_fiat_rates(sdk: BreezSdk):
   # ANCHOR: list-fiat-rates
   try:
      response = await sdk.list_fiat_rates()
   except Exception as error:
      print(error)
      raise
   # ANCHOR_END: list-fiat-rates
