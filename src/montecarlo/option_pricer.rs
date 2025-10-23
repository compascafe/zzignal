use pyo3::prelude::*;
use rand::prelude::*;
use crate::montecarlo::engine::montecarlo_mean;

/// Monte Carlo European call option pricing
#[pyfunction]
pub fn montecarlo_option(
    s0: f64,
    k: f64,
    r: f64,
    sigma: f64,
    t: f64,
    n: usize,
) -> PyResult<f64> {
    let mut rng = rand::thread_rng();
    let mut payoff_sum = 0.0;

    // Here you could use rayon later for parallel speedup
    for _ in 0..n {
        let z: f64 = rng.sample(rand_distr::StandardNormal);
        let st = s0 * f64::exp((r - 0.5 * sigma * sigma) * t + sigma * f64::sqrt(t) * z);
        let payoff = f64::max(st - k, 0.0);
        payoff_sum += payoff;
    }

    let mean_payoff = payoff_sum / (n as f64);
    let price = f64::exp(-r * t) * mean_payoff;
    Ok(price)
}

