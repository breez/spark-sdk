SUBDIRS := $(wildcard */.)

.PHONY: $(SUBDIRS)

$(SUBDIRS):
	$(MAKE) -C $@ $(MAKECMDGOALS)

default: check

build: $(SUBDIRS)

check: fmt-check clippy-check test

clippy-fix: $(SUBDIRS)

clippy-check: $(SUBDIRS)

fix: fmt-fix clippy-fix

fmt-fix: $(SUBDIRS)

fmt-check: $(SUBDIRS)

test: $(SUBDIRS)
