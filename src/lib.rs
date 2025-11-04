use pyo3::prelude::*;

pub mod synthetic;

#[pyfunction]
fn hello_world() -> PyResult<String> {
    Ok("🦀 Hello from ZZignal — Synthetic Module Active".to_string())
}

#[pymodule]
fn zzignal(_py: Python, m: &Bound<PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(hello_world, m)?)?;
    m.add_function(wrap_pyfunction!(synthetic::generate_data, m)?)?;
    m.add_function(wrap_pyfunction!(synthetic::dummy_message, m)?)?;
    Ok(())
}
