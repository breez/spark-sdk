# Boltz USDT-Lightning Integration

Swaps between Lightning (sats) and USDT via Boltz Exchange. Uses a two-hop architecture:

```
Lightning <-> tBTC (Boltz reverse swap) <-> USDT (DEX swap on Arbitrum)
```

A Router contract makes claim + DEX atomic — one EVM transaction claims tBTC from the ERC20Swap contract and executes the Uniswap V3 swap to USDT.

## Scope

**Supported:** LN -> USDT to any address on any Boltz-supported chain. Cross-chain delivery uses OFT bridging (LayerZero) initiated from Arbitrum.

**Not supported (yet):** USDT -> LN (submarine swaps), external wallet/signer integration.

## Key Decisions

| Decision | Choice |
|----------|--------|
| EVM keys | SDK-managed, derived from seed |
| Gas | Alchemy EIP-7702 — users don't need ETH |
| ABI + signing | alloy-sol-types + k256 (WASM-compatible) |
| Swap status | WebSocket (matches Boltz web app) |

## External References

- [Boltz API docs](https://api.docs.boltz.exchange)
- [Boltz web app source](https://github.com/BoltzExchange/boltz-web-app)
- [Boltz regtest environment](https://github.com/BoltzExchange/regtest)
- [boltz-core contracts](https://github.com/BoltzExchange/boltz-core)
