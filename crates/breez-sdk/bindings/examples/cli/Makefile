SYNC_PROMPTS := sync-prompts/generate.py

.PHONY: help sync-prompts-check sync-prompts-generate

help: ## Show available targets
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | awk 'BEGIN {FS = ":.*?## "}; {printf "  %-24s %s\n", $$1, $$2}'

sync-prompts-check: ## Verify generated workflow files are up-to-date
	python3 $(SYNC_PROMPTS) --check

sync-prompts-generate: ## Regenerate workflow files from templates
	python3 $(SYNC_PROMPTS)
