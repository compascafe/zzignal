use pyo3::prelude::*;
use rand::{SeedableRng, rngs::StdRng};
use rand_distr::{Normal, Distribution};

#[pyfunction]
pub fn generate_spy_intraday(
    n_minutes: usize,
    s0: f64,
    drift_ann: f64,
    vol_ann: f64,
    seed: u64,
) -> PyResult<Vec<f64>> {
    let minutes_per_year = 252.0 * 390.0;
    let dt = 1.0 / minutes_per_year;

    let mut rng = StdRng::seed_from_u64(seed);
    let normal = Normal::new(0.0, 1.0).unwrap();

    let mut s = s0;
    let mut path = Vec::with_capacity(n_minutes);

    for _ in 0..n_minutes {
        let z = normal.sample(&mut rng);
        s *= f64::exp((drift_ann - 0.5 * vol_ann * vol_ann) * dt + vol_ann * z * dt.sqrt());
        path.push(s);
    }

    Ok(path)
}
