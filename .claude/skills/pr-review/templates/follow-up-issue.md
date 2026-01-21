# Follow-up Issue Template

Use when creating follow-up issues for downstream repositories.

## Repositories

- [breez/glow](https://github.com/breez/glow) - Flutter app (use Dart code examples)
- [breez/breez-sdk-spark-example](https://github.com/breez/breez-sdk-spark-example) - Web example (use TypeScript code examples)

## Template

```markdown
> 🧪 Follow-up from [spark-sdk PR #{pr_number}](https://github.com/breez/spark-sdk/pull/{pr_number})

## Summary

{Brief description of the SDK change and why this repository needs to integrate it}

## New SDK API

```{language}
{Code example showing how to use the new API}
```

## Implementation Tasks

- [ ] {Task 1}
- [ ] {Task 2}
- [ ] {Task 3}

## References

- [{Binding} snippet]({link_to_snippet_if_exists})
```

## Labels

| Label | When to use |
|-------|-------------|
| `enhancement` | New features |
| `breaking-change` | Breaking API changes |
| `spark-sdk` | SDK-related issues |
