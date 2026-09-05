.PHONY: format format-check lint test cargo-check build stop-test-process check version tag release publish

VERSION ?=
TAG = v$(VERSION)
MESSAGE = Release $(TAG)

ifneq ($(filter version tag release,$(MAKECMDGOALS)),)
ifeq ($(strip $(VERSION)),)
$(error VERSION is required, for example: make version VERSION=1.2.2 or make tag VERSION=1.2.2)
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
	@echo "Tag $(TAG) created at the current commit. Verify CI before running 'make release VERSION=$(VERSION)'."

release:
	git push origin "$(TAG)"

publish:
	cargo publish
