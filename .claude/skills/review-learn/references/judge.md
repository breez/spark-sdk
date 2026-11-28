# Judge rules

For each human comment on the PR, compare it with what the breez-review review showed.

- **full:** a finding names the same problem in the same place or by the same
  mechanism, and its fix would settle the human's concern.
- **partial:** a finding flags the same code for a related reason, or names the
  problem without the key mechanism.
- **miss:** otherwise.

Label each human comment **S** (bug, race, data loss, scale, design, API shape,
missing binding or implementation, a test that cannot fail) or **s** (naming,
wording, docs phrasing, comment style, unused code, local refactor). Skip review
summaries like "LGTM" and comments that only point at another comment.
