# Keyshare seed

`operator-<index>.tsv` holds the signing keyshares that operator `<index>` is
seeded with when a local cluster starts, in PostgreSQL `COPY` text format. The
three files are one FROST keyshare set viewed from the three operators: row *n*
of each file is the same keyshare id, holding that operator's own secret share.

## Why it is committed

Operators fill their pool by running DKG. Every operator coordinates its own
round, each round involves all three operators, and an operator runs its rounds
one at a time. With several clusters on one CI runner the rounds outrun the
machine, and a stalled round is not retried until the operator's task timeout
expires, so a cluster can sit for minutes without usable keyshares. Seeding
skips all of it: pools start above `dkg.min_available_keys`, so the DKG task
never fires and cluster startup does not depend on it.

Reusing one set across clusters is safe because the fixture is deterministic:
operator keys, identifiers, indices and threshold are fixed constants, and each
cluster gets its own databases and its own regtest chain.

## Regenerating

```bash
make capture-itest-keyshares
```

This starts one cluster that runs real DKG, waits for the rounds to land, and
rewrites all three files. The fixtures embed the files at compile time, so if a
file is missing rather than stale, recreate it empty (`touch operator-0.tsv`)
before capturing. Commit them together: a set that disagrees across
operators is unusable, and the unit tests in `fixtures/keyshares.rs` fail if the
committed files drift apart or drop below the DKG threshold.

Regenerate after changing:

- the pinned operator version (`ARG VERSION` in `docker/*.dockerfile`), if it
  changes the `signing_keyshares` schema. A `COPY` against a changed schema
  fails the fixture rather than loading a wrong row.
- `NUM_OPERATORS`, `MIN_SIGNERS`, or the operator identity keys in
  `fixtures/spark_so.rs`. Keyshares are bound to the operator set that
  generated them.
