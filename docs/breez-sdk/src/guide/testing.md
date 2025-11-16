# Testing and Development

## Regtest Network

For most testing and development, we recommend using the **regtest network** - a deployed test network maintained by Lightspark that provides a safe environment with no real value at risk.

### What You Can Test on Regtest

- **Spark Payments**: Bitcoin and token payments using the Spark protocol
- **Deposits**: Receiving test Bitcoin from the [Lightspark Regtest Faucet](https://app.lightspark.com/regtest-faucet)
- **Withdrawals**: Sending funds back to on-chain addresses
- **Token Issuance**: Creating and testing tokens using the SDK's [issuing functionality](./issuing_tokens.md)

### Getting Started

1. [Initialize the SDK](./initializing.md) using the default regtest config (no API key required)
2. [Generate a Bitcoin receiving address](./receive_payment.md#bitcoin) 
3. Request funds from the [faucet](https://app.lightspark.com/regtest-faucet) to your generated address
4. Test all Spark-related functionality safely

## Lightning Network Testing

For Lightning payments specifically, we recommend testing on **mainnet with small amounts** since the regtest network doesn't have a developed Lightning Network.

Use real satoshis but keep transaction values very low while verifying payment flows work correctly.

## Development Best Practices

- **Start with regtest** for most development and testing
- **Use mainnet** for Lightning testing
- **Test all payment types** you plan to support in your application
