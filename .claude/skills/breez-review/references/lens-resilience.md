# Resilience & Failure-Mode Lens

## The lens, in one line

**What happens on restart, on a crash after a side effect, under a race, offline, and at
scale, and how does the system recover?**

## Always ask

1. If the process restarts in the middle of this flow, does it recover correctly, or can
   it exit early / leave partial state?
2. Is there a race here? Trace the interleaving explicitly (checks outside the lock, notify
   before await, two instances/clients acting at once).
3. If we crash right after this side effect (a send, a transfer, a webhook register), what
   picks it up next time?
4. Does this hold offline / on first run with no network, and on a flaky mobile connection?
5. Does this scale? What at tens of thousands of leaves, or a storage hit on every request?

## Signature smells to hunt

- **Partial-sync then restart.** Paging that stops early when it re-encounters known items,
  so a restart mid-sync never walks all the way back. Prefer a recorded "sync up to this id
  / this timestamp" boundary.
- **Check outside the lock.** A pre-check (has-subscription, exists) done before taking the
  lock, then acted on inside, leaving a window for another caller. Trace the exact sequence.
- **Crash after a side effect.** A network/state mutation with no idempotency and no later
  reconciliation. Ask what recovers it.
- **Left-forever state.** Pending/rolled-back rows that nothing ever cleans up.
- **Load-everything-into-memory.** Fetching all leaves/records into memory on refresh when
  the wallet can hold tens of thousands. Prefer ids, paging, or a store lookup.
- **Per-request storage / network cost.** Hitting sqlite/postgres or the network on every
  SSP request or payment; on server storage this does not scale. Prefer an in-memory value
  refreshed in the background.
- **Retry too broad or too long.** Retrying on non-transport errors, or a retry/delay that
  can stall the sync loop.

## Anchor examples (real calls, this repo)

Ground your judgment in how these were actually reviewed:

- "if we started sync, got 1-2 pages and then restarted ... it may exit early and not sync
  all the way back. One way to solve this is to maintain the newest payment id we should sync
  up to and stop there": restart-safe paging.
- "there is a small race here where we try to subscribe in the start ... because the check
  for user_has_unexpired_invoices is outside the lock": name the interleaving.
- "What is the mechanism to recover if we crash after this send?": crash-after-side-effect.
- "I have seen users with tens of thousands of leaves ... perhaps fetch only the leaf ids,
  and scope it to the signers that actually make network requests": scale on refresh.
- "This will hit the storage on every request to ssp. While sqlite is OK, on postgres it is
  not efficient. The session manager should be dumb, taking the token from memory": per-request
  cost.
