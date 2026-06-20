# USDC Solana Integration Spec

## Why Solana USDC
- Fastest blockchain (400ms block time)
- Lowest fees (< $0.01 per transfer)
- Widest stablecoin adoption in DeFi

## Integration via Jupiter
- Swap BTC to USDC using Jupiter aggregator
- Swap USDC to BTC for receiving
- All on-chain, non-custodial

## Dependencies
- solana-sdk, spl-token crates
- Jupiter API (free, no API key)

## Steps
1. Add Solana deps to Cargo.toml
2. Implement USDC transfer
3. Implement Jupiter swap
4. Add tests

See issue #634
