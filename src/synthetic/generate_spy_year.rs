use pyo3::prelude::*;
use rand::{SeedableRng, rngs::StdRng};
use rand_distr::{Normal, Distribution};

#[pyfunction]
pub fn generate_spy_year(
    n_days: usize,
    s0: f64,
    drift_ann: f64,
    vol_ann: f64,
    seed: u64,
    minutes_per_day: usize,
    strikes: Vec<f64>,
    iv_atm: f64,
    skew_alpha: f64,
    smile_beta: f64,
    r: f64,
    iv_floor: f64,
) -> PyResult<(Vec<Vec<f64>>, Vec<Vec<Vec<f64>>>)> {
    let mut rng = StdRng::seed_from_u64(seed);
    let normal = Normal::new(0.0, 1.0).unwrap();

    let minutes_per_year = 252.0 * minutes_per_day as f64;
    let dt = 1.0 / minutes_per_year;
    let mut s = s0;

    let mut all_days_spy = Vec::with_capacity(n_days);
    let mut all_days_opts = Vec::with_capacity(n_days);

    for _ in 0..n_days {
        let mut prices = Vec::with_capacity(minutes_per_day);
        for _ in 0..minutes_per_day {
            let z = normal.sample(&mut rng);
            s *= f64::exp((drift_ann - 0.5 * vol_ann * vol_ann) * dt + vol_ann * z * dt.sqrt());
            prices.push(s);
        }

        let (p_mat, _, _, _, _) = crate::synthetic::options_0dte::price_greeks_chain_0dte_call(
            prices.clone(),
            strikes.clone(),
            iv_atm,
            skew_alpha,
            smile_beta,
            r,
            minutes_per_day,
            iv_floor,
        )?;
        all_days_spy.push(prices);
        all_days_opts.push(p_mat);
    }

    Ok((all_days_spy, all_days_opts))
}
