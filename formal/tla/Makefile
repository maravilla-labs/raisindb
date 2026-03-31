# Makefile for TLA+ Model Checking
# Provides convenient commands for running TLC on all specifications

# TLC command (adjust if using JAR file directly)
TLC = tlc
TLCFLAGS = -workers auto

# Specifications
SPECS = VectorClock CausalDelivery Idempotency NetworkModel LWW AddWinsSet DeleteWins RGA

# Colors for output
GREEN = \033[0;32m
YELLOW = \033[0;33m
RED = \033[0;31m
NC = \033[0m # No Color

.PHONY: all check clean help $(SPECS)

# Default target: check all specs
all: $(SPECS)

# Check individual specifications
VectorClock:
	@echo "$(GREEN)Checking VectorClock specification...$(NC)"
	$(TLC) $(TLCFLAGS) VectorClock.tla -config VectorClock.cfg
	@echo "$(GREEN)✓ VectorClock check completed$(NC)\n"

CausalDelivery:
	@echo "$(GREEN)Checking CausalDelivery specification...$(NC)"
	$(TLC) $(TLCFLAGS) CausalDelivery.tla -config CausalDelivery.cfg
	@echo "$(GREEN)✓ CausalDelivery check completed$(NC)\n"

Idempotency:
	@echo "$(GREEN)Checking Idempotency specification...$(NC)"
	$(TLC) $(TLCFLAGS) Idempotency.tla -config Idempotency.cfg
	@echo "$(GREEN)✓ Idempotency check completed$(NC)\n"

NetworkModel:
	@echo "$(GREEN)Checking NetworkModel specification...$(NC)"
	$(TLC) $(TLCFLAGS) NetworkModel.tla -config NetworkModel.cfg
	@echo "$(GREEN)✓ NetworkModel check completed$(NC)\n"

LWW:
	@echo "$(GREEN)Checking LWW (Last-Write-Wins) specification...$(NC)"
	$(TLC) $(TLCFLAGS) LWW.tla -config LWW.cfg
	@echo "$(GREEN)✓ LWW check completed$(NC)\n"

AddWinsSet:
	@echo "$(GREEN)Checking AddWinsSet specification...$(NC)"
	$(TLC) $(TLCFLAGS) AddWinsSet.tla -config AddWinsSet.cfg
	@echo "$(GREEN)✓ AddWinsSet check completed$(NC)\n"

DeleteWins:
	@echo "$(GREEN)Checking DeleteWins specification...$(NC)"
	$(TLC) $(TLCFLAGS) DeleteWins.tla -config DeleteWins.cfg
	@echo "$(GREEN)✓ DeleteWins check completed$(NC)\n"

RGA:
	@echo "$(GREEN)Checking RGA (Replicated Growable Array) specification...$(NC)"
	$(TLC) $(TLCFLAGS) RGA.tla -config RGA.cfg
	@echo "$(GREEN)✓ RGA check completed$(NC)\n"

# Quick check with reduced state space (fast)
quick:
	@echo "$(YELLOW)Running quick checks (reduced state space)...$(NC)"
	@echo "Note: Edit .cfg files to reduce MaxOps for faster checking"
	$(MAKE) VectorClock

# Check syntax only (no model checking)
syntax:
	@echo "$(GREEN)Checking syntax of all specifications...$(NC)"
	@for spec in $(SPECS); do \
		echo "Parsing $$spec.tla..."; \
		$(TLC) -check $$spec.tla || exit 1; \
	done
	@echo "$(GREEN)✓ All syntax checks passed$(NC)"

# Clean generated files
clean:
	@echo "$(YELLOW)Cleaning generated files...$(NC)"
	rm -rf states/
	rm -f *.dot *.pdf
	rm -f MC.tla MC.cfg
	@echo "$(GREEN)✓ Clean completed$(NC)"

# Detailed check with coverage
coverage:
	@echo "$(GREEN)Running with coverage analysis...$(NC)"
	$(TLC) $(TLCFLAGS) -coverage 1 CausalDelivery.tla -config CausalDelivery.cfg

# Simulation mode (random walk)
simulate:
	@echo "$(GREEN)Running simulation mode...$(NC)"
	$(TLC) -simulate -depth 100 CausalDelivery.tla -config CausalDelivery.cfg

# Help target
help:
	@echo "$(GREEN)TLA+ Model Checking Makefile$(NC)"
	@echo ""
	@echo "Targets:"
	@echo "  $(YELLOW)all$(NC)            - Check all specifications (default)"
	@echo "  $(YELLOW)VectorClock$(NC)    - Check VectorClock.tla"
	@echo "  $(YELLOW)CausalDelivery$(NC) - Check CausalDelivery.tla"
	@echo "  $(YELLOW)Idempotency$(NC)    - Check Idempotency.tla"
	@echo "  $(YELLOW)NetworkModel$(NC)   - Check NetworkModel.tla"
	@echo "  $(YELLOW)LWW$(NC)            - Check LWW.tla (Last-Write-Wins CRDT)"
	@echo "  $(YELLOW)AddWinsSet$(NC)     - Check AddWinsSet.tla (Add-Wins Set CRDT)"
	@echo "  $(YELLOW)DeleteWins$(NC)     - Check DeleteWins.tla (Delete-Wins semantics)"
	@echo "  $(YELLOW)RGA$(NC)            - Check RGA.tla (Replicated Growable Array)"
	@echo "  $(YELLOW)quick$(NC)          - Quick check with reduced state space"
	@echo "  $(YELLOW)syntax$(NC)         - Check syntax only (no model checking)"
	@echo "  $(YELLOW)coverage$(NC)       - Run with coverage analysis"
	@echo "  $(YELLOW)simulate$(NC)       - Run simulation mode (random walk)"
	@echo "  $(YELLOW)clean$(NC)          - Remove generated files"
	@echo "  $(YELLOW)help$(NC)           - Show this help message"
	@echo ""
	@echo "Examples:"
	@echo "  make VectorClock    # Check vector clock spec"
	@echo "  make LWW            # Check Last-Write-Wins CRDT"
	@echo "  make all            # Check all specs"
	@echo "  make quick          # Fast syntax check"
	@echo ""
	@echo "Environment variables:"
	@echo "  TLC=path/to/tlc     # Override TLC command"
	@echo "  TLCFLAGS=\"-Xmx4g\"   # Override TLC flags"
