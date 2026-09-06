.PHONY: help disk prune prune-tests clean-hard build test fmt lint gen-bindings gen-bindings-check wasm-fixtures wasm-sdks-test wasm-sdks-build wasm-smoke wasm-check

# Keep this first so a bare `make` is informative.
help:
	@echo "RaisinDB — common tasks"
	@echo
	@echo "  make build        Build the workspace (debug)"
	@echo "  make test         Run the workspace test suite"
	@echo "  make fmt          cargo fmt --all"
	@echo "  make lint         cargo clippy --workspace"
	@echo
	@echo "Generated SDK bindings (from the function bindings registry):"
	@echo "  make gen-bindings        Regenerate the guest SDK bindings + WIT copies"
	@echo "  make gen-bindings-check  Fail if the committed output is stale"
	@echo
	@echo "WebAssembly:"
	@echo "  make wasm-check       Lint + unit-test the wasm work (the exact gate commands)"
	@echo "  make wasm-fixtures    Rebuild the guest test fixtures (needs wasm32-wasip2)"
	@echo "  make wasm-sdks-test   Run the three guest SDK suites natively (no wasm needed)"
	@echo "  make wasm-sdks-build  Build the example artifacts (LANGS=\"rust go ts\")"
	@echo "  make wasm-smoke       Deploy + invoke the example against a running server"
	@echo
	@echo "Disk management (target/ grows to ~40 GB on a full test build):"
	@echo "  make disk         Report target/ size, broken down by what drives it"
	@echo "  make prune        Reclaim the safely-rebuildable parts (keeps deps)"
	@echo "  make prune-tests  Also delete test executables (keeps library build)"
	@echo "  make clean-hard   cargo clean — full rebuild next time"

build:
	cargo build --workspace

test:
	cargo test --workspace --no-fail-fast

fmt:
	cargo fmt --all

lint:
	cargo clippy --workspace

# ---------------------------------------------------------------------------
# Generated bindings
#
# Every `raisin.*` host method is declared ONCE in the bindings registry. The
# WebAssembly guest SDKs (Rust/Go/TypeScript) and the SDK-local copies of the
# WIT contract are rendered from it, so a new host method reaches all three in
# one step. The output is committed; `gen-bindings-check` is the CI-safe form
# and matches the freshness tests in `cargo test -p raisin-functions --lib`.
# ---------------------------------------------------------------------------

gen-bindings:
	cargo run -p raisin-functions --bin gen-bindings

gen-bindings-check:
	cargo run -p raisin-functions --bin gen-bindings -- --check

# ---------------------------------------------------------------------------
# WebAssembly guest fixtures
#
# `fixtures/wasm-guests` is its OWN cargo workspace (the root excludes
# `fixtures/*`) targeting wasm32-wasip2. The built components are committed
# under crates/raisin-functions/src/runtime/wasm/fixtures/ and include_bytes!'d,
# so the runtime tests need no wasm toolchain. Never build these from a
# build.rs — see crates/raisin-server/build.rs for why.
# ---------------------------------------------------------------------------

wasm-fixtures:
	rustup target add wasm32-wasip2
	cd fixtures/wasm-guests && cargo build --release --target wasm32-wasip2
	./fixtures/wasm-guests/copy-fixtures.sh

# ---------------------------------------------------------------------------
# The wasm gate — run these two, in exactly these spellings
#
# Both flags below are load-bearing and have each been mis-typed once, costing
# a red sweep that named crates the change never touched:
#
#   --no-deps   Without it, clippy runs as RUSTC_WORKSPACE_WRAPPER over every
#               workspace path dependency of raisin-functions, and `-D warnings`
#               then denies THEIR pre-existing lints. The first wall is
#               raisin-storage (14 `too_many_arguments`, unmodified committed
#               code, `translations.rs:42` and friends); fixing it only moves
#               the wall, because the whole workspace carries ~305 such
#               findings. Making that gate green is a `[workspace.lints.clippy]`
#               policy decision, not something a feature branch can close.
#
#   --bins      raisin-server declares a single [[bin]] and has NO src/lib.rs,
#               so `--lib` errors out ("no library targets found") before a
#               single test runs. `--bins` is the only spelling that works.
# ---------------------------------------------------------------------------

wasm-check:
	cargo fmt --all -- --check
	cargo clippy -p raisin-functions --features wasm --no-deps -- -D warnings
	cargo test -p raisin-functions --lib --features wasm
	SKIP_ADMIN_BUILD=1 cargo test -p raisin-server --bins config

# ---------------------------------------------------------------------------
# WebAssembly guest SDKs and the example package
#
# `wasm-sdks-test` runs each SDK's NATIVE suite against its mock host: no wasm
# toolchain, no componentization, no server. That is deliberately the CI-cheap
# half — componentizing stays a local/`make` step.
#
# `wasm-sdks-build` goes through `raisindb function build`, the command users
# actually type, so the build/copy/report rule has exactly ONE implementation.
# It needs the toolchain for each language it is asked for: cargo +
# wasm32-wasip2 (rust), tinygo (go), node + jco (ts).
# ---------------------------------------------------------------------------

WASM_EXAMPLE := examples/wasm-functions
RAISINDB_CLI := packages/raisindb-cli/dist/index.js
# Which example projects `wasm-sdks-build` builds: `all`, or any of rust go ts.
LANGS ?= all

wasm-sdks-test:
	@WASM_SDKS_STRICT=$${WASM_SDKS_STRICT:-0} ./scripts/wasm-sdks-test.sh

wasm-sdks-build:
	@# A STALE dist is the trap here, not a missing one: `dist/index.js` from
	@# before the wasm commands existed runs happily and reports "unknown
	@# command", so probe for the subcommand rather than the file.
	@node $(RAISINDB_CLI) function --help >/dev/null 2>&1 || { \
		echo "The CLI at $(RAISINDB_CLI) is missing or predates \`function build\`."; \
		echo "  pnpm --filter @raisindb/cli run build"; exit 1; }
	@set -e; cd $(WASM_EXAMPLE); \
	if [ "$(LANGS)" = "all" ]; then \
		node ../../$(RAISINDB_CLI) function build --all; \
	else \
		for lang in $(LANGS); do \
			echo "── greet-$$lang"; \
			node ../../$(RAISINDB_CLI) function build "wasm/demo/greet-$$lang"; \
		done; \
	fi

# Needs a running server (RAISINDB_SERVER, default http://localhost:8080) and
# the artifacts built. See examples/wasm-functions/README.md.
wasm-smoke:
	cd $(WASM_EXAMPLE) && node smoke.mjs

# ---------------------------------------------------------------------------
# Disk
#
# A full `cargo test --workspace` reaches ~40 GB because every integration test
# file links its own binary against the whole static dependency graph (rocksdb,
# tantivy, candle, tesseract). `make disk` shows where it actually went, so the
# right thing gets deleted instead of nuking target/ and paying a 20-minute
# rebuild.
# ---------------------------------------------------------------------------

disk:
	@echo "target/ total:"
	@du -sh target 2>/dev/null || echo "  (no target/ yet)"
	@echo
	@echo "By directory:"
	@du -sh target/debug/* 2>/dev/null | sort -rh | head -8 || true
	@echo
	@echo "Inside target/debug/deps:"
	@printf "  test executables  %8s KB  (%s binaries)\n" \
		"$$(find target/debug/deps -maxdepth 1 -type f -perm +111 2>/dev/null | xargs du -ck 2>/dev/null | tail -1 | cut -f1)" \
		"$$(find target/debug/deps -maxdepth 1 -type f -perm +111 2>/dev/null | wc -l | tr -d ' ')"
	@printf "  .rlib             %8s KB\n" \
		"$$(find target/debug/deps -maxdepth 1 -name '*.rlib' 2>/dev/null | xargs du -ck 2>/dev/null | tail -1 | cut -f1)"
	@printf "  .rmeta            %8s KB\n" \
		"$$(find target/debug/deps -maxdepth 1 -name '*.rmeta' 2>/dev/null | xargs du -ck 2>/dev/null | tail -1 | cut -f1)"
	@echo
	@echo "Free space:"
	@df -h . | tail -1

# Reclaims the parts that cost the least to regenerate: incremental caches and
# any stale dSYM bundles. Library .rlib/.rmeta are kept, so the next build is
# fast.
prune:
	@echo "Removing incremental caches and debug bundles..."
	@rm -rf target/debug/incremental target/release/incremental
	@find target -name '*.dSYM' -maxdepth 3 -prune -exec rm -rf {} + 2>/dev/null || true
	@$(MAKE) --no-print-directory disk

# Also drops the test executables — the single biggest consumer. The library
# build survives, so `cargo build` stays fast; only `cargo test` relinks.
prune-tests: prune
	@echo "Removing test executables..."
	@find target/debug/deps -maxdepth 1 -type f -perm +111 -delete 2>/dev/null || true
	@$(MAKE) --no-print-directory disk

clean-hard:
	cargo clean
