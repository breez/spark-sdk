# Review checklist

What reviewers here check beyond correctness and the rules in `CLAUDE.md`.

## Bitcoin and crypto code

- Use the types and named constants that `bitcoin`, `secp256k1` and `frost`
  already provide instead of hand-written numbers, sizes and helpers.
- Do fee math with the library's fee-rate type, in the unit it uses internally.

## Naming

- One concept has one name across the code, the docs and the guide in a PR.
- A name says what the thing is. A boolean reads as a question: `has_x`, `is_x`.
- Where the SDK has several kinds of one thing, the name says which kind.

## Guide docs (`docs/breez-sdk/src/guide/`)

- A section is about as long as the sections on similar features next to it.
- It explains how to use the API, not how the SDK works inside.
- It documents only what the SDK supports today.
- A change that affects one kind of integrator (a signer type, a platform) is
  also noted on that integrator's page.
