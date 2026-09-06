.PHONY: format format-check lint test cargo-check coverage coverage-install build stop-test-process check version tag git-formal-push release publish

VERSION ?=
TAG = v$(VERSION)
MESSAGE = Release $(TAG)

ifneq ($(filter tag release,$(MAKECMDGOALS)),)
ifeq ($(strip $(VERSION)),)
ifeq ($(OS),Windows_NT)
VERSION := $(shell powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "scripts/get-version.ps1")
else
VERSION := $(shell sh scripts/get-version.sh)
endif
endif
ifeq ($(strip $(VERSION)),)
$(error Could not read the stoker-engine version from Cargo.toml)
endif
endif

ifneq ($(filter version,$(MAKECMDGOALS)),)
ifeq ($(strip $(VERSION)),)
$(error VERSION is required for make version, for example: make version VERSION=1.2.2)
endif
endif

format:
	cargo fmt --all

format-check:
	cargo fmt --all -- --check

lint:
	cargo clippy --all-targets --all-features -- -D warnings

test:
	cargo test --locked --all-targets

cargo-check:
	cargo check --all-targets --all-features

coverage:
	cargo llvm-cov --locked --all-features --workspace --html
	@echo "Coverage report: $(CURDIR)/target/llvm-cov/html/index.html"

coverage-install:
	rustup component add llvm-tools-preview --toolchain stable
	cargo +stable install cargo-llvm-cov --locked

build:
	cargo build --release

stop-test-process:
ifeq ($(OS),Windows_NT)
	powershell -NoProfile -NonInteractive -Command "$$target = [System.IO.Path]::GetFullPath('target/debug/stoker.exe'); Get-CimInstance Win32_Process | Where-Object { $$_.Name -eq 'stoker.exe' -and $$_.ExecutablePath -eq $$target } | ForEach-Object { Stop-Process -Id $$_.ProcessId -Force }"
else
	-pkill -f -- "$(CURDIR)/target/debug/stoker"
endif

check:
	$(MAKE) stop-test-process
	$(MAKE) format-check
	$(MAKE) cargo-check
	$(MAKE) lint
	$(MAKE) test
	$(MAKE) build

version:
ifeq ($(OS),Windows_NT)
	powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "scripts/set-version.ps1" -Version "$(VERSION)"
else
	sh scripts/set-version.sh "$(VERSION)"
endif

tag:
	git tag -a "$(TAG)" -m "$(MESSAGE)"
	@echo "Tag $(TAG) created at the current commit. Verify CI before running 'make release'."

git-formal-push:
ifeq ($(OS),Windows_NT)
	@powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass -Command "$$branch = (git branch --show-current).Trim(); if ($$branch -notmatch '^v[0-9]+\.[0-9]+\.[0-9]+$$') { Write-Error 'Current branch must match vX.Y.Z, for example v1.3.0'; exit 1 }; Write-Host ('Pushing formal release branch ' + $$branch); git push origin $$branch"
else
	@branch="$$(git branch --show-current)"; \
	if [ -z "$$branch" ]; then \
		echo "Current branch must match vX.Y.Z; detached HEAD is not allowed." >&2; exit 1; \
	fi; \
	if ! printf '%s\n' "$$branch" | grep -Eq '^v[0-9]+\.[0-9]+\.[0-9]+$$'; then \
		echo "Current branch '$$branch' must match vX.Y.Z, for example v1.3.0." >&2; exit 1; \
	fi; \
	echo "Pushing formal release branch $$branch"; \
	git push origin "$$branch"
endif

release:
	git push origin "$(TAG)"

publish:
	cargo publish
