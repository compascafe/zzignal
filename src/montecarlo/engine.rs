use rayon::prelude::*;
use rand::Rng;
use rand_distr::{Distribution, StandardNormal};

/// Parallel Monte Carlo core returning the mean payoff
pub fn montecarlo_mean<F>(
    n_paths: usize,
    s0: f64,
    r: f64,
    sigma: f64,
    t: f64,
    payoff: F,
) -> f64
where
    F: Fn(f64) -> f64 + Send + Sync,
{
    let sum: f64 = (0..n_paths)
        .into_par_iter()
        .map_init(
            || rand::thread_rng(),
            |rng, _| {
                let z: f64 = StandardNormal.sample(rng);
                let st = s0 * ((r - 0.5 * sigma * sigma) * t + sigma * t.sqrt() * z).exp();
                payoff(st)
            },
        )
        .sum();

    sum / n_paths as f64
}
