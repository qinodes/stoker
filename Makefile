.PHONY: format format-check lint test cargo-check web-install web-build web-test web-browser-test coverage coverage-install build stop-test-process dev-restart check version publish

VERSION ?=

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
	cargo clippy --locked --all-targets --all-features -- -D warnings

test:
	cargo test --locked --all-targets

cargo-check:
	cargo check --locked --all-targets --all-features

web-install:
	npm ci

web-build:
	npm run build
	npm run typecheck

web-test:
	npm run test:web:unit

web-browser-test:
	npm run test:web:browser

coverage:
	cargo llvm-cov --locked --all-features --workspace --html
	@echo "Coverage report: $(CURDIR)/target/llvm-cov/html/index.html"

coverage-install:
	rustup component add llvm-tools-preview --toolchain stable
	cargo +stable install cargo-llvm-cov --locked

build:
	cargo build --locked --release

stop-test-process:
ifeq ($(OS),Windows_NT)
	powershell -NoProfile -NonInteractive -Command "$$target = [System.IO.Path]::GetFullPath('target/debug/stoker.exe'); Get-CimInstance Win32_Process | Where-Object { $$_.Name -eq 'stoker.exe' -and $$_.ExecutablePath -eq $$target } | ForEach-Object { Stop-Process -Id $$_.ProcessId -Force }"
else
	-pkill -f -- "$(CURDIR)/target/debug/stoker"
endif

# Development helper for restarting the locally installed Stoker binary.
dev-restart:
	-stoker ui stop
	-stoker stop
ifeq ($(OS),Windows_NT)
	powershell -NoProfile -NonInteractive -Command "$$target = [System.IO.Path]::GetFullPath((Get-Command stoker.exe -ErrorAction Stop).Source); $$processes = @(Get-CimInstance Win32_Process | Where-Object { $$_.Name -eq 'stoker.exe' -and $$_.ExecutablePath -eq $$target }); $$processes | ForEach-Object { Stop-Process -Id $$_.ProcessId -Force }; $$deadline = (Get-Date).AddSeconds(10); do { $$remaining = @(Get-CimInstance Win32_Process | Where-Object { $$_.Name -eq 'stoker.exe' -and $$_.ExecutablePath -eq $$target }); if ($$remaining.Count -eq 0) { break }; Start-Sleep -Milliseconds 100 } while ((Get-Date) -lt $$deadline); if ($$remaining.Count -gt 0) { Write-Error ('Could not stop Stoker processes before install: ' + (($$remaining | Select-Object -ExpandProperty ProcessId) -join ', ')); exit 1 }"
else
	-pkill -f -- "$$(command -v stoker)"
endif
	cargo install --path . --locked
	stoker start && stoker ui start

check:
	$(MAKE) stop-test-process
	$(MAKE) format-check
	$(MAKE) web-build
	$(MAKE) cargo-check
	$(MAKE) lint
	$(MAKE) test
	$(MAKE) web-test
	$(MAKE) build

version:
ifeq ($(OS),Windows_NT)
	powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "scripts/set-version.ps1" -Version "$(VERSION)"
else
	sh scripts/set-version.sh "$(VERSION)"
endif

publish:
	cargo publish
