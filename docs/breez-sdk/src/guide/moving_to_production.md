# Moving to production

## Production checklist

Before moving to production, we strongly recommend verifying that these use cases are correctly implemented. Here is a checklist you can use to ensure that your application is production ready.

- **Add logging**: Add sufficient logging into your application to diagnose any issues users are having. Include log entries from the Breez SDK up to and including **DEBUG** level. For more information see [Adding logging](logging.md).
  > ⚠️ Proper logging is a prerequisite for troubleshooting. If logging is not implemented (or is implemented incorrectly), the Breez team will not be able to assist in diagnosing or resolving reported issues.

- **Display pending payments**: Payments always contain a status field that can be used to determine whether the payment was completed or not. Make sure you handle the case where the payment is still pending by showing the correct status to the user.

- **Claiming on-chain deposits**: Make sure you handle the case where an on-chain deposit is unclaimed. For more information see [Claiming on-chain deposits](onchain_claims.md).
