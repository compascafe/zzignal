use pyo3::prelude::*;
use rayon::prelude::*;

/// Hello World example
#[pyfunction]
fn hello_world() -> PyResult<String> {
    Ok("🦀 Hello from ZZignal (Rust + Python)!".to_string())
}

/// Dummy API function #1 — Compute sum of squares (parallelized)
#[pyfunction]
fn sum_of_squares(n: usize) -> PyResult<u64> {
    let result: u64 = (0..n as u64).into_par_iter().map(|x| x * x).sum();
    Ok(result)
}

/// Dummy API function #2 — Simulate Monte Carlo mean (placeholder)
#[pyfunction]
fn montecarlo_mean(samples: usize) -> PyResult<f64> {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let vals: Vec<f64> = (0..samples).map(|_| rng.gen::<f64>()).collect();
    let mean = vals.par_iter().sum::<f64>() / samples as f64;
    Ok(mean)
}

/// Python module definition
#[pymodule]
fn zzignal(py: Python, m: &Bound<PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(hello_world, m)?)?;
    m.add_function(wrap_pyfunction!(sum_of_squares, m)?)?;
    m.add_function(wrap_pyfunction!(montecarlo_mean, m)?)?;
    Ok(())
}
