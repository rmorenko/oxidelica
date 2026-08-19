# Oxidelica — code quality in one command.
#
#   make help      — list targets
#   make fmt       — apply all formatters (rust, toml, md/json/yaml)
#   make check     — everything in check mode: formats, linters, spelling,
#                    tests, coverage
#
# Tools: rustfmt, clippy, taplo, prettier (npx), markdownlint-cli2
# (npx), yamllint (uvx), jq, xmllint, typos, codespell (uvx),
# cargo-audit, cargo-machete, cargo-llvm-cov.
#
# The ones fetched rather than installed carry their version. A linter
# left to float turns a green commit red on a release that has nothing
# to do with it, and the same versions are named in the workflow, so
# what passes here passes there. Raise them deliberately.

SHELL := /bin/bash
.DEFAULT_GOAL := help

# Files by type (target/ and .git are excluded).
FIND := find . -path ./target -prune -o -path ./.git -prune -o
JSON_FILES := $(shell $(FIND) -name '*.json' -print)
XML_FILES  := $(shell $(FIND) \( -name '*.xml' -o -name '*.svg' \) -print)

.PHONY: help
help: ## show this list
	@grep -E '^[a-zA-Z_-]+:.*## ' $(MAKEFILE_LIST) | awk -F':.*## ' '{printf "  \033[36m%-14s\033[0m %s\n", $$1, $$2}'

# ---------- formatting ----------

.PHONY: fmt
fmt: ## apply all formatters
	cargo fmt --all
	taplo fmt
	npx --yes prettier@3.9.6 --write "**/*.{md,json,yaml,yml}" --log-level warn

.PHONY: fmt-check
fmt-check: ## verify formatting without changes
	cargo fmt --all -- --check
	taplo fmt --check --diff
	npx --yes prettier@3.9.6 --check "**/*.{md,json,yaml,yml}" --log-level warn

# ---------- linters ----------

.PHONY: lint
lint: lint-rust lint-toml lint-md lint-yaml lint-json lint-xml lint-cyrillic ## all linters

.PHONY: lint-rust
lint-rust: ## clippy with warnings as errors
	cargo clippy --workspace --all-targets -- -D warnings

.PHONY: lint-toml
lint-toml: ## TOML validation (taplo)
	taplo check

.PHONY: lint-md
lint-md: ## markdownlint
	npx --yes markdownlint-cli2@0.23.2 "**/*.md" "!target"

.PHONY: lint-yaml
lint-yaml: ## yamllint (when yaml files exist)
	@files=$$($(FIND) \( -name '*.yaml' -o -name '*.yml' \) -print); \
	if [ -n "$$files" ]; then uvx yamllint@1.38.0 -s $$files; else echo "yaml: no files"; fi

.PHONY: lint-json
lint-json: ## JSON validation (jq)
	@ok=1; for f in $(JSON_FILES); do \
	  jq empty "$$f" || { echo "invalid JSON: $$f"; ok=0; }; \
	done; [ $$ok -eq 1 ]
	@echo "json: ok ($(words $(JSON_FILES)) files)"

.PHONY: lint-xml
lint-xml: ## XML/SVG validation (xmllint)
	@if [ -n "$(XML_FILES)" ]; then xmllint --noout $(XML_FILES) && echo "xml: ok"; \
	else echo "xml: no files"; fi

.PHONY: lint-cyrillic
lint-cyrillic: ## language rule: no Cyrillic outside *.ru.md and locales/ru.conf
	python3 scripts/check_cyrillic.py

# ---------- spelling ----------

.PHONY: spell
spell: ## spell check (typos + codespell)
	typos
	uvx codespell@2.4.3 --config .codespellrc

.PHONY: spell-fix
spell-fix: ## auto-fix typos
	typos --write-changes
	uvx codespell@2.4.3 --config .codespellrc --write-changes

# ---------- dependencies ----------

.PHONY: audit
audit: ## vulnerabilities (cargo-audit) and unused deps (cargo-machete)
	cargo audit
	cargo machete

# ---------- other platforms ----------

.PHONY: linux-check
linux-check: ## build and test the core on Linux in Docker
	docker run --rm -v "$$PWD":/src -w /src -e CARGO_TARGET_DIR=/tmp/target \
	  -v oxidelica-cargo:/usr/local/cargo/registry rust:slim \
	  cargo test -p oxidelica-parser -p oxidelica-sim -p oxidelica-cli

.PHONY: windows-check
windows-check: ## cross-build, lint and run the test suite for Windows under Wine
	cargo build --target x86_64-pc-windows-gnu --workspace
	cargo clippy --workspace --all-targets --target x86_64-pc-windows-gnu -- -D warnings
	CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUNNER=wine \
	  cargo test --workspace --target x86_64-pc-windows-gnu
	@echo "note: needs mingw-w64 and wine (brew install mingw-w64; brew install --cask wine-stable)"

.PHONY: linux-check-ide
linux-check-ide: ## build the GUI on Linux in Docker (pulls Bevy's system libraries)
	docker run --rm -v "$$PWD":/src -w /src -e CARGO_TARGET_DIR=/tmp/target \
	  -v oxidelica-cargo:/usr/local/cargo/registry rust:slim bash -c \
	  "apt-get update -qq && apt-get install -y -qq pkg-config libasound2-dev libudev-dev \
	   && cargo build -p oxidelica-ide"

# ---------- tests and coverage ----------

.PHONY: test
test: ## all tests
	cargo test --workspace

.PHONY: cov
cov: ## core coverage with the 95% line threshold
	./scripts/coverage.sh --summary-only

.PHONY: cov-report
cov-report: ## HTML coverage report (opens in the browser)
	cargo llvm-cov -p oxidelica-parser -p oxidelica-sim -p oxidelica-cli --html
	@echo "report: target/llvm-cov/html/index.html"
	@if command -v open > /dev/null; then open target/llvm-cov/html/index.html; \
	 elif command -v xdg-open > /dev/null; then xdg-open target/llvm-cov/html/index.html; fi

.PHONY: cov-lcov
cov-lcov: ## lcov coverage output (for CI)
	cargo llvm-cov -p oxidelica-parser -p oxidelica-sim -p oxidelica-cli \
	  --lcov --output-path target/lcov.info
	@echo "report: target/lcov.info"

# ---------- aggregates ----------

.PHONY: check
check: fmt-check lint spell audit test cov ## full check (as in CI)
	@echo "OK: all checks passed"

.PHONY: ci
ci: check ## alias for check
