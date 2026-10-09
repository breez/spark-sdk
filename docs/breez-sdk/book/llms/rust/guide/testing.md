# Testing and development

## Regtest Network

For most testing and development, we recommend using the **Regtest Network** - a deployed test network maintained by Lightspark that is free to use and carries no real-world value.

### What you can test on Regtest

- **Spark Payments**: Bitcoin and token payments using the Spark protocol
- **Deposits**: Receiving test Bitcoin from the [Lightspark Regtest Faucet](https://app.lightspark.com/regtest-faucet)
- **Withdrawals**: Sending funds back to on-chain addresses
- **Token Issuance**: Creating and testing tokens using the SDK's [issuing functionality](./issuing_tokens.md)

### Getting started

1. [Initialize the SDK](./initializing.md) using the default regtest config (no API key required)
2. [Generate a Bitcoin receiving address](./receive_payment.md#bitcoin)
3. Request funds from the [faucet](https://app.lightspark.com/regtest-faucet) to your generated address
4. Test all Spark-related functionality in a controlled development environment

## Mainnet testing

Some features rely on live networks that Regtest doesn't reproduce. Test these on **Mainnet with small amounts**: use real satoshis, but keep transaction values very low while verifying the flows work correctly.

### Lightning payments

The Regtest Network doesn't have a developed Lightning Network, so test Lightning send and receive flows on Mainnet.

### Stable balance and USDC/USDT

The stablecoin assets are only available on Mainnet:

- **USDB** is the Spark-native stablecoin behind [Stable Balance](./stable_balance.md).
- **USDC** and **USDT** are cross-chain assets. Use [USDC/USDT](./cross_chain.md) to pay recipients on their native chains or receive from them. The cross-chain providers operate against live external networks and have no testnet equivalent.

Test these integrations on Mainnet with small amounts.

## Development best practices

- **Start with Regtest** for most development and testing
- **Use Mainnet** for Lightning, stable balance, and USDC/USDT testing
- **Test all payment types** you plan to support in your application
