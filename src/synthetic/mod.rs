use pyo3::prelude::*;

pub mod etf_intraday;
pub mod options_0dte;
pub mod generate_spy_year;
pub mod save_csv;

#[pyfunction]
pub fn dummy_message(name: String) -> PyResult<String> {
    Ok(format!("Hello, {}! 🦀 Synthetic module ready.", name))
}

#[pymodule]
pub fn synthetic(_py: Python, m: &Bound<PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(dummy_message, m)?)?;
    m.add_function(wrap_pyfunction!(etf_intraday::generate_spy_intraday, m)?)?;
    m.add_function(wrap_pyfunction!(options_0dte::price_greeks_chain_0dte_call, m)?)?;
    m.add_function(wrap_pyfunction!(generate_spy_year::generate_spy_year, m)?)?;
    m.add_function(wrap_pyfunction!(save_csv::save_spy_to_csv, m)?)?;
    Ok(())
}
