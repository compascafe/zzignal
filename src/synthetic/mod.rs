use pyo3::prelude::*;

#[pyfunction]
pub fn generate_data(n: usize) -> PyResult<Vec<f64>> {
    let data = (0..n).map(|i| (i as f64).sin()).collect();
    Ok(data)
}

#[pyfunction]
pub fn dummy_message(name: String) -> PyResult<String> {
    Ok(format!("Hello, {}! This is a dummy function from ZZignal Synthetic 🦀", name))
}
