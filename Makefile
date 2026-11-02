# Builds CreateCalculated from source. Needs Rust (https://rustup.rs), a C compiler and make.
# `make help` lists the targets.

VERSION := $(shell sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)
PROGRAMS := createcalc-server createcalc-recipes createcalc-icons createcalc-import
RELEASE := target/release
ADDRESS ?= 127.0.0.1:3000
PREFIX ?= /opt/createcalc

# `make dist` links everything statically so the result runs on any x86-64 Linux.
DIST_TARGET ?= x86_64-unknown-linux-musl
DIST_NAME := createcalc-$(VERSION)
DIST_ARCHIVE := dist/$(DIST_NAME)-linux-x86_64.tar.gz
# The ring crate needs a C compiler. Plain gcc does, as long as it does not call glibc's checked functions.
DIST_ENV := CC_x86_64_unknown_linux_musl=$${CC_x86_64_unknown_linux_musl:-$$(command -v musl-gcc || echo gcc)} \
	CFLAGS_x86_64_unknown_linux_musl="-U_FORTIFY_SOURCE -D_FORTIFY_SOURCE=0"

.PHONY: help build test run data install dist vercel clean

help:
	@echo "make build    build the programs into $(RELEASE)"
	@echo "make test     run all tests"
	@echo "make run      start the server on ADDRESS, default $(ADDRESS)"
	@echo "make data     download game files and build recipes and pictures in data/"
	@echo "make install  install to PREFIX, default $(PREFIX), with a service when run as root"
	@echo "make dist     build a static archive for Linux servers in dist/"
	@echo "make vercel   upload a test version to Vercel"
	@echo "make clean    remove build results, data/ is kept"

build:
	cargo build --release --workspace --bins

test:
	cargo test --release --workspace

run: build
	$(RELEASE)/createcalc-server data/bundle.json $(ADDRESS) data/icons

data: build
	$(RELEASE)/createcalc-recipes
	$(RELEASE)/createcalc-icons

install: build
	packaging/install.sh --from $(RELEASE) --prefix $(PREFIX)

dist:
	rustup target add $(DIST_TARGET)
	$(DIST_ENV) cargo build --release --workspace --bins --target $(DIST_TARGET)
	rm -rf dist/$(DIST_NAME) $(DIST_ARCHIVE) $(DIST_ARCHIVE).sha256
	mkdir -p dist/$(DIST_NAME)
	cd target/$(DIST_TARGET)/release && strip $(PROGRAMS) && cp $(PROGRAMS) $(CURDIR)/dist/$(DIST_NAME)/
	cp README.md packaging/install.sh packaging/createcalc.service dist/$(DIST_NAME)/
	tar -C dist -czf $(DIST_ARCHIVE) $(DIST_NAME)
	cd dist && sha256sum $(DIST_NAME)-linux-x86_64.tar.gz > $(DIST_NAME)-linux-x86_64.tar.gz.sha256
	@echo "$(DIST_ARCHIVE)"

vercel:
	packaging/vercel.sh

clean:
	cargo clean
	rm -rf dist
