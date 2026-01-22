<h1 id="lnurl-auth">
    <a class="header" href="#lnurl-auth">Using LNURL-Auth</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.lnurl_auth">API docs</a>
</h1>

LNURL-Auth allows users to authenticate with services using their Lightning app, without requiring passwords or usernames. The Breez SDK supports LNURL-Auth following the LUD-04 and LUD-05 specifications.

## How it works

LNURL-Auth uses cryptographic key derivation to generate domain-specific keys, ensuring that:
- Each service gets a unique authentication key
- Your master key remains private
- Authentication is secure and passwordless

The SDK handles:
1. Domain-specific key derivation (LUD-05)
2. Challenge signing
3. Callback to the LNURL service

## Parsing LNURL-Auth URLs

After [parsing](parse.md) an LNURL-Auth URL, you'll receive an `LnurlAuthRequestDetails` object containing:
- **k1** - The authentication challenge (hex-encoded 32 bytes)
- **action** - Optional action type: `register`, `login`, `link`, or `auth`
- **domain** - The service domain requesting authentication
- **url** - The callback URL

{{#tabs lnurl_auth:parse-lnurl-auth}}

## Performing Authentication

Once you have the authentication request details, you can perform the authentication by passing the request to the `lnurl_auth` method. The SDK will:
1. Derive a domain-specific key pair
2. Sign the challenge with the derived key
3. Send the signature and public key to the service

{{#tabs lnurl_auth:lnurl-auth}}

<div class="warning">
<h4>Developer note</h4>
The SDK automatically derives domain-specific keys according to LUD-05, ensuring that each service gets a unique linking key. This protects user privacy by preventing services from correlating user identities across different domains.
</div>

## Action Types

LNURL-Auth supports different action types that indicate the purpose of the authentication:

- **register** - Create a new account
- **login** - Sign in to an existing account
- **link** - Link the Lightning wallet to an existing account
- **auth** - Generic authentication

Your application can use the `action` field to provide appropriate UI feedback to users.

## Security Considerations

- Always verify the domain before authenticating
- Show the domain to users for confirmation
- The SDK derives unique keys per domain to prevent tracking
- Authentication keys cannot be used to access funds

## Supported Specs

- [LUD-01](https://github.com/lnurl/luds/blob/luds/01.md) LNURL bech32 encoding
- [LUD-04](https://github.com/lnurl/luds/blob/luds/04.md) `auth` base spec
- [LUD-05](https://github.com/lnurl/luds/blob/luds/05.md) BIP32-based seed generation for `auth`
- [LUD-17](https://github.com/lnurl/luds/blob/luds/17.md) Support for lnurl auth 
