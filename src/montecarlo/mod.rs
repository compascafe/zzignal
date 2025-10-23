mod path;
pub(crate) mod option_pricer;
mod engine;
mod timer;

use pyo3::prelude::*;
use rand::prelude::*;
use rand_distr::StandardNormal;

/// Simulates the stock price at time T using geometric Brownian motion
pub fn simulate_stock_price(s0: f64, r: f64, sigma: f64, t: f64, z: f64) -> f64 {
    let drift = (r - 0.5 * sigma * sigma) * t;
    let diffusion = sigma * f64::sqrt(t) * z;
    s0 * f64::exp(drift + diffusion)
}

/// Calculates the payoff of a European call option
pub fn calculate_call_payoff(stock_price: f64, strike_price: f64) -> f64 {
    f64::max(stock_price - strike_price, 0.0)
}

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

    for _ in 0..n {
        let z: f64 = rng.sample(StandardNormal);
        let st = simulate_stock_price(s0, r, sigma, t, z);
        let payoff = calculate_call_payoff(st, k);
        payoff_sum += payoff;
    }

    let mean_payoff = payoff_sum / n as f64;
    let price = f64::exp(-r * t) * mean_payoff;
    Ok(price)
}
