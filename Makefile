# Unsupervised: common development commands.
# `make check` is the gate. It must pass before every merge.

.DEFAULT_GOAL := help
JS_PACKAGES := web sdk/ts agents/llm

.PHONY: help
help: ## Show this help
	@grep -E '^[a-zA-Z_-]+:.*?## ' $(MAKEFILE_LIST) | awk 'BEGIN {FS = ":.*?## "}; {printf "  %-14s %s\n", $$1, $$2}'

.PHONY: check
check: fmt-check lint test js py ## Run the full gate

.PHONY: fmt
fmt: ## Format the Rust code
	cargo fmt --all

.PHONY: fmt-check
fmt-check: ## Check the Rust format
	cargo fmt --all -- --check

.PHONY: lint
lint: ## Run clippy with warnings as errors
	cargo clippy --workspace --all-targets -- -D warnings

.PHONY: test
test: ## Run all Rust tests
	cargo test --workspace

.PHONY: js
js: ## Install, build and test every JavaScript package that exists
	@set -e; for p in $(JS_PACKAGES); do \
		if [ -f $$p/package.json ]; then \
			echo "== $$p"; \
			(cd $$p && npm ci --no-audit --no-fund && npm run build && npm test); \
		fi; \
	done

.PHONY: py
py: ## Test the Python SDK and the scripted agent
	@set -e; for p in sdk/python agents/scripted; do \
		if [ -d $$p/tests ]; then \
			echo "== $$p"; \
			(cd $$p && python3 -m unittest discover -s tests -v); \
		fi; \
	done

.PHONY: run
run: ## Start the whole stack on this machine
	cargo run --release -p unsup-server -- run

.PHONY: bench
bench: ## Run the in-process benchmark
	cargo run --release -p unsup-loadtest -- bench

.PHONY: schema
schema: ## Regenerate the protocol schema and the action catalogue
	cargo run -p unsup-server -- export-schema --out schema
