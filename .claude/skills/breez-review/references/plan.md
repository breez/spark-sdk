# Plan

Read the diff stat, the PR description, `CLAUDE.md` and `REVIEW.md`, then the files
that carry the change. Produce four things and keep them for the passes and the report.

1. **Overview** (8 lines at most): what the change lets the app or user do, why,
   and 3 to 5 one-line bullets on how, naming the real components.
2. **Sequence diagram** (medium and large only): one Mermaid `sequenceDiagram` per
   flow the change adds or changes, 1 to 3 in all. Real component and function
   names, in call order, with what data or keys move on each hop.
3. **Risk areas:** 2 to 5 one-line bullets, each a file and what to check there.
4. **File groups and themes.** Group the changed files by concern. Note which of
   these themes apply, so the passes can look for them:
   - **Old and new flow parity:** the change adds a variant of a flow that already
     exists (split, deferred, client-signed, v2). Compare them step by step:
     validation, amount and fee math, side effects (events, observers, renewals),
     persisted values, retries, post-conditions.
   - **Parallel implementations:** the change touches one of N implementations of
     an interface (Storage backends, tree stores, bindings, language CLIs, mirrored
     models). Every implementation needs the same semantics, and new shared tests
     need wiring into every harness.
   - **Two-phase flows:** something built in one call is used in a later call,
     possibly after a round trip through a client. Re-check time-sensitive facts
     at the second phase, define retry and replay, and treat returned data as
     untrusted.
   - **After the point of no return:** steps that run after a broadcast, publish or
     submit. Local failures there need one consistent policy.
   - **Refactor leftovers:** fields never set or read, derives nothing uses, tests of
     removed behaviour, helpers the refactor meant to merge.
