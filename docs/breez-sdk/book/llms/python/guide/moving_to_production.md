# Moving to production

## Production checklist

Before moving to production, we strongly recommend verifying that these use cases are correctly implemented. Here is a checklist you can use to ensure that your application is production ready.

- **Add logging**: Add sufficient logging into your application to diagnose any issues users are having. Include log entries from the Breez SDK up to and including **DEBUG** level. For more information see [Adding logging](logging.md).
  > ⚠️ Proper logging is a prerequisite for troubleshooting. If logging is not implemented (or is implemented incorrectly), the Breez team will not be able to assist in diagnosing or resolving reported issues.

- **Display pending payments**: Payments always contain a status field that can be used to determine whether the payment was completed or not. Make sure you handle the case where the payment is still pending by showing the correct status to the user.

- **Handling unclaimed on-chain deposits**: The SDK claims a deposit on its own only while the claim costs no more than the [max claim fee](onchain_claims.md#the-max-claim-fee). When rising on-chain fees push the claim fee above it, the deposit stays unclaimed and the SDK emits `SdkEvent.UNCLAIMED_DEPOSITS`. Without handling this, the user's funds look stuck. Build one of these on top of the automatic claims:
  - **Manual claiming**: show the user the claim fee, ask for approval, then claim with a max fee that covers it. See [Claiming deposits manually](onchain_manual_claims.md).
  - **Custom claim logic**: claim by your own rules without asking the user, such as claiming only above an amount or under a fee limit capped in sats. See [Writing your own claim logic](onchain_manual_claims.md#writing-your-own-claim-logic).

  Also let users [refund](onchain_refunds.md) a deposit they don't want to claim, and show deposit states as described in [Tracking deposits](onchain_deposits.md#deposit-states).

- **Check Spark status**: Use the Spark status check to monitor the operational status of the Spark network and display a warning to users when services are degraded or experiencing issues. For more information see [Spark status](spark_status.md).

- **Server-side deployments**: If you're running the SDK on a server, review [Running on a server](server_deployments.md) for the multi-user and treasury configurations and for keeping the keys out of the server.
