use rayon::prelude::*;
use rand::Rng;

pub fn montecarlo_mean(n: usize) -> f64 {
    let samples: Vec<f64> = (0..n)
        .into_par_iter()
        .map(|_| {
            let mut r = rand::thread_rng();
            r.gen::<f64>()
        })
        .collect();
    samples.iter().sum::<f64>() / n as f64
}
