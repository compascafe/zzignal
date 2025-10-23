use pyo3::prelude::*;

pub mod core;
pub mod montecarlo;
pub mod stochastic;
pub mod options;
pub mod volatility;
pub mod utils;

use crate::montecarlo::montecarlo_option;

#[pyfunction]
fn hello_world() -> PyResult<String> {
    Ok("🦀 Hello from ZZignal — Rust Quant Engine".to_string())
}

#[pymodule]
fn zzignal(_py: Python, m: &Bound<PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(hello_world, m)?)?;
    m.add_function(wrap_pyfunction!(montecarlo::option_pricer::montecarlo_option, m)?)?;

    Ok(())
}
