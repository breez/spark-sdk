# Simplicity & YAGNI Lens

## The lens, in one line

**Is this the simplest thing that solves the problem in front of us, without machinery
for a use case that does not exist yet?**

## Always ask

1. Is this the simplest approach that works now, or are we building for a hypothetical?
2. Are we generalizing (a trait, a plugin point, an enum of strategies) before a second
   concrete use case exists?
3. Can we remove this lock, layer, wrapper, or enum and get the same result?
4. If a cheaper mechanism gives the same guarantee (e.g. idempotency instead of a
   distributed lock), why keep the expensive one?
5. Does this abstraction earn its keep, or is it speculative flexibility?

## Signature smells to hunt

- **Premature generalization.** A trait/plugin/strategy introduced with a single
  implementation. Prefer waiting for the second implementation before abstracting.
- **A lock that idempotency could replace.** Distributed/advisory locks where a
  deterministic id or a "first writer wins" already guarantees correctness.
- **Speculative extensibility.** A builder with nothing to inject, dependency-injection
  seams with one wiring, config knobs no one asked for.
- **Layers that can be flattened.** A wrapper client around a client, an enum wrapping a
  trait that is only ever one variant.
- **"Might need it later" fields/branches** that add surface without a present caller.
- **Complex query/logic where a simpler shape reads the same.** If a flatter query or a
  straight branch is clearer, prefer it.

## Anchor examples (real calls, this repo)

Ground your judgment in how these were actually reviewed:

- "I would rather wait for another implementation before attempting to generalize as a
  plugin. This has complexities also around integration with the payment list, sync etc,
  perhaps it is too soon": defer generalization until the second use case.
- "if we can achieve the same result without the distributed lock then it is definitely
  better. It seems the first one to create the transfer wins": prefer idempotency over a lock.
- "I am wondering if we need the builder at this phase since we are not letting the user
  inject any dependencies": no speculative seams.
- "It seems to me that we can flatten the layers here and avoid SigningLockClient": remove
  a wrapping layer.
- "FWIW I prefer simplicity over speed on the unilateral exit flow, so I am ok with
  sequential queries": simplicity beats a speculative optimization on a rare path.
