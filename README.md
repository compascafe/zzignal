# 🦀 ZZignal — Rust Quant Engine

**ZZignal** is an open-source quantitative finance engine written in **Rust**, with seamless **Python bindings** via [PyO3](https://pyo3.rs).  
It combines *speed*, *safety*, and *clarity* for research, trading, and education.

---

## 🚀 Features

- ⚙️ **Monte Carlo Simulation** for option pricing (fast Rust core)
- 💰 **European Options Module** — payoffs, Greeks, analytics
- 📈 **Future:** Volatility Surfaces · Stochastic Models · Risk Metrics
- 🧠 **Python Integration** — ready for Jupyter / Colab
- 📚 **Learning / Examples** — practical notebooks for quants & students

---

## 🧭 Roadmap

| Milestone | Feature | Status         |
|------------|----------|----------------|
| v0.1.x | Core Monte Carlo Engine |  🧩 In Progress            |
| v0.2.x | Options Module (Greeks · Payoff API) | 🧩 In Progress |
| v0.3.x | Volatility Models (Local / Heston) | 🔜 Planned     |
| v0.4.x | Portfolio Simulation & Risk Metrics | 🔜 Planned     |
| v1.0.0 | Docs & Full Quant Engine Release | 🔜 Future      |

See [ROADMAP.md](./ROADMAP.md) for details.

---

## 🧑‍💻 Contributing

Contributions are welcome!  
You can help by adding features, improving docs, or sharing examples.

1. **Fork** the repo and create a branch
2. **Write clean Rust or Python code**
3. **Expose** functions with `#[pyfunction]`
4. **Add examples** in `/examples`
5. **Open a PR** with a short description

👉 See [CONTRIBUTING.md](./CONTRIBUTING.md) for guidelines.

---

## 🧰 Installation

```bash
pip install zzignal
# or latest development version
pip install git+https://github.com/compascafe/zzignal.git
