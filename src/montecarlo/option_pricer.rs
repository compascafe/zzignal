use pyo3::prelude::*;
use crate::montecarlo::{engine, timer::Timer};

/// Monte Carlo European call option pricing (parallel)
#[pyfunction]
pub fn montecarlo_option(
    s0: f64,
    k: f64,
    r: f64,
    sigma: f64,
    t: f64,
    n: usize,
) -> PyResult<f64> {
    println!(
        "Running {n} Monte Carlo paths with Rayon threads..."
    );

    let mut timer = Timer::start();
    let payoff = |st: f64| (st - k).max(0.0);

    let mean_payoff = engine::montecarlo_mean(n, s0, r, sigma, t, payoff);
    let price = (-r * t).exp() * mean_payoff;
    let elapsed = timer.stop();

    println!("Monte Carlo option price: {:.6}", price);
    println!("Runtime: {:.3} seconds", elapsed);
    Ok(price)
}

