# Passes

Every pass is one subagent. Give each the base SHA, the tree path, the PR title and
description, the risk areas, the applicable themes, and these rules:

- Read the change with `git diff <base>` (in CI, `git diff <base>...HEAD`) and read
  whole files in the tree when you need context. Read-only: change nothing.
- Return findings only, one per problem: severity (Blocking, Should-fix, Nit),
  `file:line`, a one-line title, the problem in at most 2 sentences, and a fix in
  at most 3 sentences or a short code block. Quote the line you mean. An empty list
  is a fine answer.
- Report how many of the changed files you read.

## bugs

Invoke the built-in `code-review` skill with args
`medium the change shown by "git diff <base>" in <tree path>, uncommitted edits included`
(in CI: `medium` on the PR's diff). If the skill is not available, review the diff
for correctness bugs and broken `CLAUDE.md` rules yourself and give each finding a
concrete failure scenario.

## deep-thread

One reviewer, no lens. "Read the PR description, then pick the riskiest flow this
change touches and follow it end to end: every function it calls, across crates,
into dependencies under `~/.cargo/registry` when behaviour depends on them. Check
that the code does what the description says. Then do the same for the next
riskiest flow while time allows." This pass finds what the lenses miss because it
follows one path all the way down.

## design

Apply both `references/lens-architecture.md` and `references/lens-simplicity.md`:
logic in the right layer, one source of truth, and the simplest thing that works now.

## api

Apply `references/lens-api.md`: the public surface is as small as it can be, and the
integrator does as little work as possible. Include the bindings mirrors named in
`CLAUDE.md` "Updating SDK Interfaces".

## resilience

Apply `references/lens-resilience.md`: restart mid-flow, crash after a side effect,
races, offline start, flaky networks, and wallets with many leaves.

## checklist

Read `REVIEW.md` at the repo root and check the diff against each item, plus the
rules in `CLAUDE.md` (comment style, storage implementations, interface mirroring,
error variants, docs macros). Report which items passed, failed or did not apply.

## file-group owner (large changes only)

One owner per file group from the plan, until every changed file belongs to some
pass. Each owner reads every file in its group in full and reviews it like the
deep-thread pass, scoped to the group. For parallel implementations, give one owner
all implementations of the same interface and have it compare them method by method.
