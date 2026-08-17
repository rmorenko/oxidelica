# Oxidelica — качество кода одной командой.
#
#   make help      — список целей
#   make fmt       — применить все форматеры (rust, toml, md/json/yaml)
#   make check     — всё в режиме проверки: форматы, линтеры, спеллинг, тесты, покрытие
#
# Инструменты: rustfmt, clippy, taplo, prettier (npx), markdownlint-cli2,
# yamllint (uvx), jq, xmllint, typos, codespell (uvx), cargo-audit,
# cargo-machete, cargo-llvm-cov.

SHELL := /bin/bash
.DEFAULT_GOAL := help

# файлы по типам (target/ и .git исключены)
FIND := find . -path ./target -prune -o -path ./.git -prune -o
JSON_FILES := $(shell $(FIND) -name '*.json' -print)
XML_FILES  := $(shell $(FIND) \( -name '*.xml' -o -name '*.svg' \) -print)

.PHONY: help
help: ## показать этот список
	@grep -E '^[a-zA-Z_-]+:.*## ' $(MAKEFILE_LIST) | awk -F':.*## ' '{printf "  \033[36m%-14s\033[0m %s\n", $$1, $$2}'

# ---------- форматирование ----------

.PHONY: fmt
fmt: ## применить все форматеры
	cargo fmt --all
	taplo fmt
	npx --yes prettier --write "**/*.{md,json,yaml,yml}" --log-level warn

.PHONY: fmt-check
fmt-check: ## проверить форматирование без изменений
	cargo fmt --all -- --check
	taplo fmt --check --diff
	npx --yes prettier --check "**/*.{md,json,yaml,yml}" --log-level warn

# ---------- линтеры ----------

.PHONY: lint
lint: lint-rust lint-toml lint-md lint-yaml lint-json lint-xml ## все линтеры

.PHONY: lint-rust
lint-rust: ## clippy со строгими предупреждениями
	cargo clippy --workspace --all-targets -- -D warnings

.PHONY: lint-toml
lint-toml: ## валидация TOML (taplo)
	taplo check

.PHONY: lint-md
lint-md: ## markdownlint
	markdownlint-cli2 "**/*.md" "!target"

.PHONY: lint-yaml
lint-yaml: ## yamllint (если есть yaml-файлы)
	@files=$$($(FIND) \( -name '*.yaml' -o -name '*.yml' \) -print); \
	if [ -n "$$files" ]; then uvx yamllint -s $$files; else echo "yaml: файлов нет"; fi

.PHONY: lint-json
lint-json: ## валидация JSON (jq)
	@ok=1; for f in $(JSON_FILES); do \
	  jq empty "$$f" || { echo "невалидный JSON: $$f"; ok=0; }; \
	done; [ $$ok -eq 1 ]
	@echo "json: ок ($(words $(JSON_FILES)) файлов)"

.PHONY: lint-xml
lint-xml: ## валидация XML/SVG (xmllint)
	@if [ -n "$(XML_FILES)" ]; then xmllint --noout $(XML_FILES) && echo "xml: ок"; \
	else echo "xml: файлов нет"; fi

# ---------- спеллинг ----------

.PHONY: spell
spell: ## проверка орфографии (typos + codespell)
	typos
	uvx codespell --config .codespellrc

.PHONY: spell-fix
spell-fix: ## автоисправление опечаток
	typos --write-changes
	uvx codespell --config .codespellrc --write-changes

# ---------- зависимости ----------

.PHONY: audit
audit: ## уязвимости (cargo-audit) и неиспользуемые зависимости (cargo-machete)
	cargo audit
	cargo machete

# ---------- тесты и покрытие ----------

.PHONY: test
test: ## все тесты
	cargo test --workspace

.PHONY: cov
cov: ## покрытие ядра с порогом 95% строк
	./scripts/coverage.sh --summary-only

.PHONY: cov-report
cov-report: ## HTML-отчёт по покрытию (и открыть в браузере)
	cargo llvm-cov -p oxidelica-parser -p oxidelica-sim -p oxidelica-cli --html
	@echo "отчёт: target/llvm-cov/html/index.html"
	open target/llvm-cov/html/index.html

.PHONY: cov-lcov
cov-lcov: ## покрытие в формате lcov (для CI)
	cargo llvm-cov -p oxidelica-parser -p oxidelica-sim -p oxidelica-cli \
	  --lcov --output-path target/lcov.info
	@echo "отчёт: target/lcov.info"

# ---------- агрегаты ----------

.PHONY: check
check: fmt-check lint spell audit test cov ## полная проверка (как в CI)
	@echo "✅ все проверки пройдены"

.PHONY: ci
ci: check ## синоним check
