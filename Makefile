# =========================================
# ZZignal - Python Test Automation
# =========================================

VENV = .venv

# 🧩 Run Python tests using maturin
test:
	source $(VENV)/bin/activate && maturin develop && pytest python_tests/

# 🧱 Setup: create virtual env and install dependencies
init:
	python3 -m venv $(VENV)
	source $(VENV)/bin/activate && pip install maturin pytest
