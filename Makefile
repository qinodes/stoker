.PHONY: format format-check lint test cargo-check build stop-test-process check versioning release publish

VERSION ?=
TAG = v$(VERSION)
MESSAGE = Release $(TAG)

ifneq ($(filter versioning,$(MAKECMDGOALS)),)
ifeq ($(strip $(VERSION)),)
$(error VERSION is required, for example: make release VERSION=0.5.0)
endif
endif

format:
	cargo fmt --all

format-check:
	cargo fmt --all -- --check

lint:
	cargo clippy --all-targets --all-features -- -D warnings

test:
	cargo test --all

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

versioning:
	git commit --allow-empty -m "$(MESSAGE)"
	git tag -a "$(TAG)" -m "$(MESSAGE)"
	@echo "Versioning completed. Run 'make release' to publish the release."

release:
	git push origin main --follow-tags

publish:
	cargo publish
