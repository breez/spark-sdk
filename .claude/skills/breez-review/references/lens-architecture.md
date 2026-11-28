# Architecture Lens

## The lens, in one line

**Is every piece of this change in the layer that owns it, with one source of truth,
and does the wallet stay a thin coordinator?**

## Always ask

1. Does this logic belong in the service that owns the domain, or is it leaking into
   the wallet / caller / storage?
2. Is there exactly one source of truth for this state, or are we introducing a second
   view that can drift?
3. Is the wallet staying a thin layer that coordinates use cases, rather than growing
   implementation detail?
4. Does this new struct / trait / enum have a clear, single responsibility, or is it a
   bucket that "shared logic will get pushed into"?
5. Is a lower layer querying data the caller already has (and could pass down)?

## Signature smells to hunt

- **Business logic in the wallet** that should sit in a dedicated service. Ask whether
  the two or three functions want to be extracted into their own `<feature>.rs` service.
- **Duplicated queries across layers.** A lower layer re-fetches nodes/leaves/transfers
  the caller already holds. Push the fetch up to the owning service and pass the data down.
- **Domain logic leaking into the storage trait** (sync or business concerns showing up as
  storage methods or callbacks). Consider a dedicated trait (e.g. a `SyncStorage`) so the
  boundary is explicit.
- **Config that holds behavior.** Config should be a dumb data struct: connect/build logic
  belongs on the component (e.g. the pool), not on config.
- **A new abstraction with no clear job.** A struct/trait introduced to hold "shared" code
  but without a crisp responsibility. Name the responsibility or don't add it.
- **An API exposed at the wrong altitude.** For example a paging or low-level method on a
  service when no caller has a real use for it.

## Anchor examples (real calls, this repo)

Ground your judgment in how these were actually reviewed:

- "the whole leaf manipulation should be encapsulated inside the service such that the
  wallet is a thin layer that coordinates these use cases": push logic out of the wallet.
- "do you think it makes sense to extract the unilateral exit code into its own
  `unilateral_exit.rs` service? Just looking to remove the details from the wallet as much
  as we can": the service made it obvious the API didn't belong on the tree service.
- "keep the leaves/nodes fetching at the tree service only ... the lower layers won't have
  to query for nodes and instead they will get the nodes/leaves as arguments": single owner
  for fetching.
- "I'd like to think of the config as just a dummy struct that holds data": behavior off config.
- "my concern is that this new struct doesn't have a clear responsibility or purpose, it
  seems that all the shared logic will be pushed there": resist purposeless abstractions.
