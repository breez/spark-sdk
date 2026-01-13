# Glow Follow-up Issue Template

Use this template when creating follow-up issues on [breez/glow](https://github.com/breez/glow) for Flutter binding changes.

## Before Creating

**Check for existing issues first** to avoid duplicates:
```bash
gh issue list --repo breez/glow --search "{feature_name}" --state open
```

If an issue exists, update it instead of creating a new one:
```bash
gh issue edit {issue_number} --repo breez/glow --body "$(cat <<'EOF'
{updated_body}
EOF
)"
```

## Issue Template

```markdown
🧪 Follow-up from spark-sdk PR review.

---

## Summary

{Brief description of the SDK change and why Glow needs to integrate it}

## New SDK API

```dart
{Dart code example showing how to use the new API}
```

## Implementation Tasks

- [ ] {Task 1 - e.g., Add UI component}
- [ ] {Task 2 - e.g., Handle new state}
- [ ] {Task 3 - e.g., Update error handling}

## References

- [SDK PR #{pr_number}](https://github.com/breez/spark-sdk/pull/{pr_number})
- [Flutter snippet]({link_to_snippet_if_exists})
```

## Creating the Issue

```bash
gh issue create --repo breez/glow \
  --title "feat: integrate {feature_name} from spark-sdk" \
  --body "$(cat <<'EOF'
🧪 Follow-up from spark-sdk PR review.

---

## Summary
...
EOF
)"
```

## Labels

Consider adding appropriate labels:
- `enhancement` for new features
- `breaking-change` for breaking API changes
- `spark-sdk` to track SDK-related issues
