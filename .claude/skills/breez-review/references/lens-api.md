# API & Developer-Experience Lens

## The lens, in one line

**Is the public surface as small as it can be, and does the caller do as little work
as possible?**

## Always ask

1. Can this be non-public? If it is only used internally, should visibility drop to
   `pub(crate)`?
2. Can we extend an existing method (an optional parameter) instead of adding a new one?
3. Does this trait ask implementors for the minimum, or is it pushing avoidable work onto
   everyone who implements it?
4. Does the naming make the intent and the cost obvious (e.g. that a call hits the network,
   or interacts with a remote service)?
5. Are we pushing work onto the integrator that the SDK could do for them?

## Signature smells to hunt

- **Needless `pub`.** A type/function/field exposed that only has internal callers.
- **A new method where a param would do.** Adding `create_foo_with_x` instead of an optional
  argument on `create_foo`. Prefer extending, and treat `None` as the sensible default.
- **Fat traits.** New required trait methods that most implementors will not need. Keep the
  trait minimal; a rarely-needed capability can be optional or a separate trait.
- **Work dumped on the caller.** Interfaces that make the integrator assemble low-level pieces
  (leaves, fee math, multi-step flows) the SDK could compute internally in a prepare step.
- **Naming that hides intent.** A method whose name does not signal a remote call, a mutation,
  or a "set vs partial-update" distinction. Names are UX.
- **A second entry point for one concept.** Two functions that do almost the same job; prefer
  one, or fold one into the other.

## Anchor examples (real calls, this repo)

Ground your judgment in how these were actually reviewed:

- "Should we reduce visibility to crate level? I see it is only used internally": default to
  the smallest visibility that works.
- "instead of adding another function we should add an optional publicKey parameter to the
  existing create_lightning_invoice. If passed None we will use the pubkey from the signer":
  extend, don't add.
- "the signature part seems to me like an implementation detail of our current service,
  perhaps it shouldn't be in the trait definition but only in the implementation": keep the
  trait minimal for implementors.
- "This seems a lot of work needed to be done by the user. Can the SDK calculate this cost
  internally and decide which leaves should be included?": offload work from the integrator.
- "I just liked the suggestion of payment_request. I never had too much sympathy to
  'destination'": naming matters, pick the term the caller understands.
