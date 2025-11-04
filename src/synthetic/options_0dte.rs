use pyo3::prelude::*;
use std::f64::consts::PI;
use statrs::function::erf::erf;

fn n_pdf(x: f64) -> f64 { (1.0 / (2.0 * PI).sqrt()) * (-0.5 * x * x).exp() }
fn n_cdf(x: f64) -> f64 { 0.5 * (1.0 + erf(x / std::f64::consts::SQRT_2)) }

fn d1_d2(s: f64, k: f64, r: f64, sigma: f64, t: f64) -> (f64, f64) {
    let vsqrt = sigma * t.sqrt();
    let d1 = ((s / k).ln() + (r + 0.5 * sigma * sigma) * t) / vsqrt;
    let d2 = d1 - vsqrt;
    (d1, d2)
}

fn bs_call(s: f64, k: f64, r: f64, sigma: f64, t: f64) -> f64 {
    if t <= 0.0 { return (s - k).max(0.0); }
    let (d1, d2) = d1_d2(s, k, r, sigma, t);
    s * n_cdf(d1) - (k * (-r * t).exp()) * n_cdf(d2)
}

#[pyfunction]
pub fn price_greeks_chain_0dte_call(
    prices: Vec<f64>,
    strikes: Vec<f64>,
    iv_atm: f64,
    skew_alpha: f64,
    smile_beta: f64,
    r: f64,
    minutes_to_expiry: usize,
    iv_floor: f64,
) -> PyResult<(
    Vec<Vec<f64>>, Vec<Vec<f64>>, Vec<Vec<f64>>, Vec<Vec<f64>>, Vec<Vec<f64>>
)> {
    let minutes_per_year = 252.0 * 390.0;
    let n = prices.len();

    let mut p_mat = vec![vec![0.0; n]; strikes.len()];
    let mut d_mat = vec![vec![0.0; n]; strikes.len()];
    let mut g_mat = vec![vec![0.0; n]; strikes.len()];
    let mut v_mat = vec![vec![0.0; n]; strikes.len()];
    let mut th_mat = vec![vec![0.0; n]; strikes.len()];

    for (t_idx, &s) in prices.iter().enumerate() {
        let minutes_left = if minutes_to_expiry > t_idx { minutes_to_expiry - t_idx } else { 0 };
        let tau = (minutes_left as f64) / minutes_per_year;

        for (j, &k) in strikes.iter().enumerate() {
            let m = (k / s).ln();
            let sigma = (iv_atm * (1.0 + skew_alpha * m + smile_beta * m * m)).max(iv_floor);

            let (d1, d2) = d1_d2(s, k, r, sigma, tau);
            let n_d1 = n_pdf(d1);
            let price = bs_call(s, k, r, sigma, tau);
            let delta = n_cdf(d1);
            let gamma = n_d1 / (s * sigma * tau.sqrt());
            let vega  = s * n_d1 * tau.sqrt();
            let theta = - (s * n_d1 * sigma) / (2.0 * tau.sqrt())
                - r * k * (-r * tau).exp() * n_cdf(d2);

            p_mat[j][t_idx]  = price;
            d_mat[j][t_idx]  = delta;
            g_mat[j][t_idx]  = gamma;
            v_mat[j][t_idx]  = vega;
            th_mat[j][t_idx] = theta;
        }
    }

    Ok((p_mat, d_mat, g_mat, v_mat, th_mat))
}
