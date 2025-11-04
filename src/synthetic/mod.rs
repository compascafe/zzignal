use pyo3::prelude::*;

/// Existing function
#[pyfunction]
pub fn generate_data(n: usize) -> PyResult<Vec<f64>> {
    let data: Vec<f64> = (0..n).map(|i| (i as f64).sin()).collect();
    Ok(data)
}

/// 🧪 New dummy function
#[pyfunction]
pub fn dummy_message(name: String) -> PyResult<String> {
    Ok(format!("Hello, {}! This is a dummy function from ZZignal Synthetic 🦀", name))
}
