SUBDIRS := crates
EXAMPLE_SUBDIRS := examples

.PHONY: $(SUBDIRS) $(EXAMPLE_SUBDIRS)

$(SUBDIRS):
	$(MAKE) -C $@ $(MAKECMDGOALS)

$(EXAMPLE_SUBDIRS):
	$(MAKE) -C $@ $(MAKECMDGOALS)

default: check

build: $(SUBDIRS) $(EXAMPLE_SUBDIRS)

build-wasm: $(SUBDIRS)

build-release: $(SUBDIRS)

check: fmt-check clippy-check test

clippy-fix: $(SUBDIRS) $(EXAMPLE_SUBDIRS)

clippy-check: $(SUBDIRS) $(EXAMPLE_SUBDIRS)

fix: fmt-fix clippy-fix

fmt-fix: $(SUBDIRS) $(EXAMPLE_SUBDIRS)

fmt-check: $(SUBDIRS) $(EXAMPLE_SUBDIRS)

test: $(SUBDIRS) $(EXAMPLE_SUBDIRS)

wasm-clippy-check: $(SUBDIRS)