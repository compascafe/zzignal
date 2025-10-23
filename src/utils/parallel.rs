use rayon::prelude::*;

pub fn sum_parallel(v: &[f64]) -> f64 {
    v.par_iter().sum()
}
