.PHONY: format format-check lint test cargo-check build check versioning release publish

VERSION ?=
TAG = v$(VERSION)
MESSAGE = Release $(TAG)

ifneq ($(filter versioning,$(MAKECMDGOALS)),)
ifeq ($(strip $(VERSION)),)
$(error VERSION is required, for example: make release VERSION=0.4.2)
endif
endif

format:
	cargo fmt --all

format-check:
	cargo fmt --all -- --check

lint:
	cargo clippy --all-targets --all-features -- -D warnings

test:
	cargo test --all -- --test-threads=1

cargo-check:
	cargo check --all-targets --all-features

build:
	cargo build --release

check:
	$(MAKE) format-check
	$(MAKE) cargo-check
	$(MAKE) lint
	$(MAKE) test
	$(MAKE) build

versioning:
	git commit --allow-empty -m "$(MESSAGE)"
	git tag -a "$(TAG)" -m "$(MESSAGE)"

release:
	git push origin main --follow-tags

publish:
	cargo publish
