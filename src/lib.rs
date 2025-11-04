use pyo3::prelude::*;

pub mod synthetic;

#[pyfunction]
fn hello_world() -> PyResult<String> {
    Ok("🦀 Hello from ZZignal — Synthetic Quant Engine Active".to_string())
}

#[pymodule]
fn zzignal(_py: Python, m: &Bound<PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(hello_world, m)?)?;

    // ✅ Create and register the synthetic submodule correctly
    let sub = PyModule::new_bound(_py, "synthetic")?;
    synthetic::synthetic(_py, &sub)?;
    m.add_submodule(&sub)?;

    Ok(())
}
