# Using Turnkey

[Turnkey](https://www.turnkey.com/) keeps the wallet's keys inside a secure enclave. The SDK ships Turnkey-backed signers, so a server can run wallets without holding any signing key material, apart from the [keys exported from Turnkey](#key-exports) for some features: signing happens inside Turnkey, and what the server holds is an API credential whose permissions you control with Turnkey policies.

Turnkey is meant for [server-side deployments](server_deployments.md). Depending on the policy you attach to the server's credential, it supports two ways of sending payments: the server signs everything itself, or each payment is approved by the end user via [Client signing](client_signing.md).

The SDK connects to an existing Spark wallet in your Turnkey organization or sub-organization. Creating the wallet itself is done with Turnkey directly and is out of the SDK's scope (see [Turnkey's Spark support](https://docs.turnkey.com/features/networks/spark) for creating the wallet account).

## Connecting

Create the signers with `createTurnkeySigner` and connect with `connectWithSigner`, the same way as any [external signer](external_signer.md):

```typescript
const turnkeyConfig: TurnkeyConfig = {
  baseUrl: undefined,
  organizationId: '<turnkey sub-organization id>',
  apiPublicKey: '<api public key hex>',
  apiPrivateKey: '<api private key hex>',
  walletId: '<turnkey wallet id>',
  network: 'mainnet',
  accountNumber: undefined,
  // Set after the first connect to make later signer setup network-free
  identityPublicKey: undefined,
  retry: undefined,
  maxRps: undefined
}

const signers = await createTurnkeySigner(turnkeyConfig)

const config = defaultConfig('mainnet')
config.apiKey = '<breez api key>'

const sdk = await connectWithSigner(
  config,
  signers.breezSigner,
  signers.sparkSigner,
  'breez_spark_db' // For WASM, this is the IndexedDB database name
)
```



A few notes on the configuration:

- `apiPrivateKey` is a server secret. It authenticates every Turnkey request; keep it out of client code and logs.
- The API key pair can be secp256k1 or P-256 (Turnkey's console default). All published bindings support both. If you use the Rust crate directly, enable the `turnkey` cargo feature, plus `turnkey-p256` for P-256 keys.
- `maxRps` paces requests to Turnkey. Unset uses Turnkey's documented limit of 10 requests per second per sub-organization; set it if your account has a different limit.

### Reconnecting without network calls

Server deployments often build a fresh SDK instance per request. Setting `identityPublicKey` makes the signer setup network-free. Obtain it once from a freshly built signer with `getIdentityPublicKey` (available right after `createTurnkeySigner`, no connect needed), or from `identityPubkey` on `getInfo` if you already have a connected SDK. Store it alongside the wallet and pass it in the config on later connects. It is a stable, non-secret value, but it must belong to the same wallet.

Without it, every signer setup also asks Turnkey to create the wallet's identity accounts. Once they exist, Turnkey records these requests as failed activities (the path already exists in the wallet account). The failures are expected and harmless, and setting `identityPublicKey` avoids them.

### Wallets under a deny-export policy

If your Turnkey policy denies key exports, or allows only the static deposit key (see [Key exports](#key-exports)), use `createTurnkeySigningOnlySigner` instead: it never needs the encryption key. Connect its signers as described in [Signers Without Local ECIES/HMAC Support](external_signer.md#signers-without-local-ecieshmac-support), which lists the features a signing-only signer can't use. Bitcoin deposits still need the static deposit key: without its export, `receivePayment` fails for a Bitcoin address, so the SDK never issues an address it could not claim or refund.

## Signing models

How payments are authorized is decided by the Turnkey policy attached to each credential, not by SDK code. Configure the policies in Turnkey; see the [Turnkey policy documentation](https://docs.turnkey.com/concepts/policies/overview) for the mechanics and [Turnkey's Spark operations](https://docs.turnkey.com/features/networks/spark#supported-operations) for the activity names the policy scopes.

Whichever model you choose, the credential also needs to create wallet accounts (`CREATE_WALLET_ACCOUNTS`): the SDK adds the accounts it needs to the wallet, such as the identity accounts and the accounts of the [exported keys](#key-exports).

### Server-side signing

The server's API credential is allowed to run all Spark signing activities. Every SDK flow then works as documented, with the exception listed under [Availability](#availability), starting with [Sending payments](send_payment.md): the server prepares, signs and sends on its own. Use this when the server is trusted to send payments autonomously.

### User-approved payments

The policy allows the server's credential to run everything except the transfer approval activity (`SPARK_PREPARE_TRANSFER`), which requires the end user's own Turnkey credential (for example a passkey registered with Turnkey). The server then drives the send with the [Client signing](client_signing.md) flow: it prepares the payment and builds the package, the user approves and signs the one item that needs their credential, and the server publishes it. The rest of the signing (`SPARK_SIGN_FROST`) stays with the server under policy, so a payment can never be sent without the user, and no key leaves Turnkey on either side, except the static deposit key for [Bitcoin deposits](#key-exports).

## Key exports

Two keys leave the enclave, each exported with `EXPORT_WALLET_ACCOUNT` the first time a feature needs it:

| Key | Path | Needed by | Used for |
|---|---|---|---|
| Static deposit key | `m/8797555'/<account>'/3'/0'` | both Turnkey signers | [Bitcoin deposits](receive_payment.md#bitcoin): issuing the address, [claiming](onchain_claims.md) and [refunding](onchain_refunds.md) |
| Encryption key | `m/8797555'/<account>'/2147483647'` | `createTurnkeySigner` only | [Real-time sync](config.md#real-time-sync-server-url) (on by default), [cross-chain payments](cross_chain.md) and [LNURL-auth](lnurl_auth.md) |

`<account>` is the wallet's account number: 1 on mainnet unless you set `accountNumber`.

The static deposit key controls only funds sent to the wallet's Bitcoin deposit address. The encryption key controls no Spark funds, but it encrypts the data the SDK syncs and, with cross-chain payments, the secrets of swaps in progress.

### Allowing only these exports

Allow these two paths in your policies rather than denying exports: in Turnkey a deny always overrides an allow, so a policy that denies `EXPORT_WALLET_ACCOUNT` also blocks these keys. Turnkey denies anything no policy allows, so every other key and the wallet seed stay in the enclave. Policies apply only to non-root users (Turnkey's root quorum bypasses them), so the server's credential must belong to a non-root user.

The static deposit key, needed with either signer. Turnkey's [Spark policy examples](https://docs.turnkey.com/features/policies/examples/spark#allow-exporting-only-static-deposit-keys) publish the same policy without the account check:

```json
{
  "policyName": "Allow exporting the static deposit key",
  "effect": "EFFECT_ALLOW",
  "consensus": "approvers.any(user, user.id == '<DELEGATED_USER_ID>')",
  "condition": "activity.kind == 'EXPORT_WALLET_ACCOUNT' && wallet_account.path_indexes.count() == 4 && wallet_account.path_indexes[0] == 8797555 && wallet_account.path_indexes[1] == 1 && wallet_account.path_indexes[2] == 3 && wallet_account.path_hardened.all(h, h == true)"
}
```

The encryption key, needed with `createTurnkeySigner`:

```json
{
  "policyName": "Allow exporting the Breez SDK encryption key",
  "effect": "EFFECT_ALLOW",
  "consensus": "approvers.any(user, user.id == '<DELEGATED_USER_ID>')",
  "condition": "activity.kind == 'EXPORT_WALLET_ACCOUNT' && wallet_account.path_indexes.count() == 3 && wallet_account.path_indexes[0] == 8797555 && wallet_account.path_indexes[1] == 1 && wallet_account.path_indexes[2] == 2147483647 && wallet_account.path_hardened.all(h, h == true)"
}
```

Replace `<DELEGATED_USER_ID>` with the id of the non-root Turnkey user that holds the server's credential, and the `1` in `path_indexes[1]` with your account number if it differs. Keep these conditions in policies of their own: a policy that reads the path fields matches only exports.

## Availability

- Turnkey signers are available on all platforms except Flutter, which does not support external signers (see [Using an External Signer](external_signer.md)).
- In the Rust crate the integration is behind the `turnkey` cargo feature. The published bindings ship with it enabled.
- The [watchtower fee ladder](config.md#watchtower-fee-ladder) is not supported with a Turnkey signer. A ladder adds many signatures, which Turnkey bills one by one, and a leaf the operators defend with it can need the cooperative recovery described below. Setting `watchtowerFeeLadderEnabled` to true fails to connect.
- A Turnkey wallet cannot recover funds cooperatively yet: Turnkey does not support the signing round a [cooperative recovery](recover_funds.md) needs, so every `RecoveryMethod.Cooperative` leaf comes back in `failed` with `CooperativeRecoveryError.Generic`.
