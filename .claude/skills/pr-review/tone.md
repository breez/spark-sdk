# Review Tone Guidelines

## Feedback Style

| Approach | Example |
|----------|---------|
| Question intent | "Was removing the timeout intentional?" |
| Probe edge cases | "What happens if this returns None?" |
| Challenge necessity | "Is this Default needed? It can panic if X." |
| Suggest alternatives | "Could we simplify this by...?" |

Phrase feedback as questions when unsure - this invites discussion rather than creating friction.

## Severity Indicators

When categorizing issues, use clear severity levels:

| Level | Meaning | Example |
|-------|---------|---------|
| **Blocking** | Prevents merge | Security vulnerability, breaking change |
| **Important** | Should be addressed | Bug, significant improvement needed |
| **Suggestion** | Nice to have | Style preference, minor optimization |

## Recommendations

Use these standard recommendation outcomes for reviews:

| Recommendation | Meaning |
|----------------|---------|
| **APPROVE** | Sound design, correct implementation |
| **REQUEST CHANGES** | Blocking issues must be addressed |
| **COMMENT** | Feedback only, non-blocking |

## Before Approving

Include at least one substantive observation or question. A review that only summarizes the code provides limited value.

## For Serious Issues

| Do | Reason |
|----|--------|
| State the issue and proposed fix | Actionable feedback moves things forward |
| Ask if the behavior change was intentional | Assumes good intent, opens discussion |
| Reference specific lines or code | Concrete examples clarify the concern |
