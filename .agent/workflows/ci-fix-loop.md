---
description: Monitor CI run, check for errors, fix them, and iterate until passing
---

# CI Fix Iteration Workflow

This workflow automates the process of monitoring a CI run, checking for errors, fixing them, and iterating until CI passes.

## Prerequisites
- GitHub CLI (`gh`) must be installed and authenticated
- You must be in the repository directory

## Usage
When a CI run is in progress, tell the agent:
"Monitor CI run <RUN_ID> on breez/spark-sdk and fix any errors"

Or after pushing changes:
"Watch the latest CI run and fix any errors that come up"

## Steps

// turbo-all

### 1. Get the latest run ID (if not provided)
```bash
gh run list --repo breez/spark-sdk --limit 1 --json databaseId --jq '.[0].databaseId'
```

### 2. Check if the run is still in progress
```bash
gh run view <RUN_ID> --repo breez/spark-sdk --json status --jq '.status'
```
If status is "in_progress" or "queued", wait 30-60 seconds and check again.

### 3. Once completed, check the conclusion
```bash
gh run view <RUN_ID> --repo breez/spark-sdk --json conclusion --jq '.conclusion'
```
If conclusion is "success", stop - CI passed!

### 4. If failed, get the error logs
```bash
gh run view <RUN_ID> --repo breez/spark-sdk --log-failed 2>&1 | grep -E "(error|Error:|failed|undefined)" | head -100
```

### 5. Analyze errors and fix them
Based on the error output:
- Fix the identified issues in the codebase
- Common patterns:
  - Import/module errors: Check package names against working files
  - Type errors: Check field names and access patterns
  - Lint errors: Follow the linter suggestions

### 6. Commit and push fixes
```bash
git add -A && git commit -m "fix: address CI errors" && git push
```

### 7. Get the new run ID and repeat from step 2
```bash
gh run list --repo breez/spark-sdk --limit 1 --json databaseId --jq '.[0].databaseId'
```

## Notes
- The agent will parse error messages and fix code automatically
- Each iteration should address specific errors found in the logs
- The loop continues until CI passes or no more fixable errors are found
