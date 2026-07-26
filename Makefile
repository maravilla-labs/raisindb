.PHONY: help disk prune prune-tests clean-hard build test fmt lint

# Keep this first so a bare `make` is informative.
help:
	@echo "RaisinDB — common tasks"
	@echo
	@echo "  make build        Build the workspace (debug)"
	@echo "  make test         Run the workspace test suite"
	@echo "  make fmt          cargo fmt --all"
	@echo "  make lint         cargo clippy --workspace"
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
