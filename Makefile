.PHONY: format format-check lint test cargo-check web-install web-build web-test web-browser-test coverage coverage-install build stop-test-process dev-restart check version tag preview-tag verify-release-branch verify-preview-tag git-release-push preview release publish

VERSION ?=
TAG = v$(VERSION)
MESSAGE = Release $(TAG)
PREVIEW ?= 1
PREVIEW_TAG = preview/$(TAG)-$(PREVIEW)
PREVIEW_MESSAGE = Preview $(PREVIEW_TAG)

ifneq ($(filter tag preview-tag release git-release-push preview,$(MAKECMDGOALS)),)
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

verify-release-branch:
ifeq ($(OS),Windows_NT)
	@powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass -Command "$$branch = (git branch --show-current).Trim(); if ($$branch -eq 'main') { exit 0 }; if ($$branch -notmatch '^release/v([0-9]+\.[0-9]+\.[0-9]+)$$') { Write-Error 'Current branch must be main or release/vX.Y.Z, for example release/v2.0.0'; exit 1 }; if ($$Matches[1] -ne '$(VERSION)') { Write-Error ('Release branch version ' + $$Matches[1] + ' does not match package version $(VERSION).'); exit 1 }"
else
	@branch="$$(git branch --show-current)"; \
	if [ -z "$$branch" ]; then \
		echo "Current branch must be main or release/vX.Y.Z; detached HEAD is not allowed." >&2; exit 1; \
	fi; \
	if [ "$$branch" = "main" ]; then exit 0; fi; \
	if ! printf '%s\n' "$$branch" | grep -Eq '^release/v[0-9]+\.[0-9]+\.[0-9]+$$'; then \
		echo "Current branch '$$branch' must be main or release/vX.Y.Z, for example release/v2.0.0." >&2; exit 1; \
	fi; \
	branch_version="$${branch#release/v}"; \
	if [ "$$branch_version" != "$(VERSION)" ]; then \
		echo "Release branch version $$branch_version does not match package version $(VERSION)." >&2; exit 1; \
	fi
endif

verify-preview-tag: verify-release-branch
ifeq ($(OS),Windows_NT)
	@powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass -Command "if ('$(PREVIEW)' -notmatch '^[1-9][0-9]*$$') { Write-Error 'PREVIEW must be a positive integer, for example PREVIEW=1.'; exit 1 }; git rev-parse --verify --quiet 'refs/tags/$(PREVIEW_TAG)' *> $$null; if ($$LASTEXITCODE -ne 0) { Write-Error 'Preview tag $(PREVIEW_TAG) does not exist. Run make preview-tag PREVIEW=$(PREVIEW) first.'; exit 1 }; $$tagCommit = (git rev-list -n 1 '$(PREVIEW_TAG)').Trim(); $$headCommit = (git rev-parse HEAD).Trim(); if ($$tagCommit -ne $$headCommit) { Write-Error 'Preview tag $(PREVIEW_TAG) does not point to the current commit.'; exit 1 }"
else
	@case "$(PREVIEW)" in ''|0|*[!0-9]*) echo "PREVIEW must be a positive integer, for example PREVIEW=1." >&2; exit 1;; esac; \
	if ! git rev-parse --verify --quiet "refs/tags/$(PREVIEW_TAG)" >/dev/null 2>&1; then \
		echo "Preview tag $(PREVIEW_TAG) does not exist. Run make preview-tag PREVIEW=$(PREVIEW) first." >&2; exit 1; \
	fi; \
	tag_commit="$$(git rev-list -n 1 "$(PREVIEW_TAG)")"; \
	head_commit="$$(git rev-parse HEAD)"; \
	if [ "$$tag_commit" != "$$head_commit" ]; then \
		echo "Preview tag $(PREVIEW_TAG) does not point to the current commit." >&2; exit 1; \
	fi
endif

preview-tag: verify-release-branch
ifeq ($(OS),Windows_NT)
	@powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass -Command "if ('$(PREVIEW)' -notmatch '^[1-9][0-9]*$$') { Write-Error 'PREVIEW must be a positive integer, for example PREVIEW=1.'; exit 1 }; $$worktreeStatus = git status --porcelain; if ($$LASTEXITCODE -ne 0) { Write-Error 'Could not inspect the working tree.'; exit 1 }; if ($$worktreeStatus) { Write-Error 'Working tree must be clean before creating $(PREVIEW_TAG).'; exit 1 }; git rev-parse --verify --quiet 'refs/tags/$(PREVIEW_TAG)' *> $$null; if ($$LASTEXITCODE -eq 0) { Write-Error 'Tag $(PREVIEW_TAG) already exists.'; exit 1 }; git tag -a '$(PREVIEW_TAG)' -m '$(PREVIEW_MESSAGE)'"
else
	@case "$(PREVIEW)" in ''|0|*[!0-9]*) echo "PREVIEW must be a positive integer, for example PREVIEW=1." >&2; exit 1;; esac; \
	worktree_status="$$(git status --porcelain)"; \
	if [ -n "$$worktree_status" ]; then \
		echo "Working tree must be clean before creating $(PREVIEW_TAG)." >&2; exit 1; \
	fi; \
	if git rev-parse --verify --quiet "refs/tags/$(PREVIEW_TAG)" >/dev/null 2>&1; then \
		echo "Tag $(PREVIEW_TAG) already exists." >&2; exit 1; \
	fi; \
	git tag -a "$(PREVIEW_TAG)" -m "$(PREVIEW_MESSAGE)"
endif
	@echo "Preview tag $(PREVIEW_TAG) created at the current commit. Push it with 'make preview PREVIEW=$(PREVIEW)'."

tag: verify-preview-tag
ifeq ($(OS),Windows_NT)
	@powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass -Command "$$worktreeStatus = git status --porcelain; if ($$LASTEXITCODE -ne 0) { Write-Error 'Could not inspect the working tree.'; exit 1 }; if ($$worktreeStatus) { Write-Error 'Working tree must be clean before creating $(TAG).'; exit 1 }; git rev-parse --verify --quiet 'refs/tags/$(TAG)' *> $$null; if ($$LASTEXITCODE -eq 0) { Write-Error 'Tag $(TAG) already exists.'; exit 1 }; git tag -a '$(TAG)' -m '$(MESSAGE)'"
else
	@worktree_status="$$(git status --porcelain)"; \
	if [ -n "$$worktree_status" ]; then \
		echo "Working tree must be clean before creating $(TAG)." >&2; exit 1; \
	fi; \
	if git rev-parse --verify --quiet "refs/tags/$(TAG)" >/dev/null 2>&1; then \
		echo "Tag $(TAG) already exists." >&2; exit 1; \
	fi; \
	git tag -a "$(TAG)" -m "$(MESSAGE)"
endif
	@echo "Tag $(TAG) created at the current commit. Push it with 'make release' to promote the preview artifact."

preview: verify-preview-tag
ifeq ($(OS),Windows_NT)
	@powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass -Command "Write-Host ('Pushing preview tag ' + '$(PREVIEW_TAG)'); git push origin ('refs/tags/' + '$(PREVIEW_TAG)')"
else
	@echo "Pushing preview tag $(PREVIEW_TAG)"; \
	git push origin "refs/tags/$(PREVIEW_TAG)"
endif

git-release-push: verify-release-branch
ifeq ($(OS),Windows_NT)
	@powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass -Command "$$branch = (git branch --show-current).Trim(); if ($$branch -notmatch '^release/v[0-9]+\.[0-9]+\.[0-9]+$$') { Write-Error 'Current branch must match release/vX.Y.Z, for example release/v2.0.0'; exit 1 }; Write-Host ('Pushing release branch ' + $$branch); git push origin $$branch"
else
	@branch="$$(git branch --show-current)"; \
	if [ -z "$$branch" ]; then \
		echo "Current branch must match release/vX.Y.Z; detached HEAD is not allowed." >&2; exit 1; \
	fi; \
	if ! printf '%s\n' "$$branch" | grep -Eq '^release/v[0-9]+\.[0-9]+\.[0-9]+$$'; then \
		echo "Current branch '$$branch' must match release/vX.Y.Z, for example release/v2.0.0." >&2; exit 1; \
	fi; \
	echo "Pushing release branch $$branch"; \
	git push origin "$$branch"
endif

release: verify-preview-tag
ifeq ($(OS),Windows_NT)
	@powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass -Command "$$tagRef = 'refs/tags/$(TAG)'; git rev-parse --verify --quiet $$tagRef *> $$null; if ($$LASTEXITCODE -ne 0) { Write-Error 'Tag $(TAG) does not exist. Run make tag first.'; exit 1 }; $$tagCommit = (git rev-list -n 1 '$(TAG)').Trim(); $$headCommit = (git rev-parse HEAD).Trim(); if ($$tagCommit -ne $$headCommit) { Write-Error 'Tag $(TAG) does not point to the current commit.'; exit 1 }; git push origin ('refs/tags/' + '$(TAG)')"
else
	@tag_commit="$$(git rev-list -n 1 "$(TAG)" 2>/dev/null)" || { echo "Tag $(TAG) does not exist. Run make tag first." >&2; exit 1; }; \
	head_commit="$$(git rev-parse HEAD)"; \
	if [ "$$tag_commit" != "$$head_commit" ]; then \
		echo "Tag $(TAG) does not point to the current commit." >&2; exit 1; \
	fi; \
	git push origin "refs/tags/$(TAG)"
endif

publish:
	cargo publish
