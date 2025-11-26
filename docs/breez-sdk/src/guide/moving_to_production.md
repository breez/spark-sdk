# Moving to production

## Production checklist

Before moving to production, there are some use cases we highly recommend you to verify you have implemented correctly. Here is a checklist you can use to ensure that your application is production ready.

- **Add logging**: Add sufficient logging into your application to diagnose any issues users are having. Include log entries from the Breez SDK up to and including **DEBUG** level. For more information see [Adding logging](logging.md).

- **Display pending payments**: Payments always contain a status field that can be used to determine if the payment was completed or not. Make sure you handle the case where the payment is still pending by showing the correct status to the user.

- **Claiming on-chain deposits**: Make sure you handle the case where an on-chain deposit is unclaimed. For more information see [Claiming on-chain deposits](onchain_claims.md).