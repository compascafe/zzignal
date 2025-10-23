use rand_distr::{Normal, Distribution};

pub fn simulate_brownian_path(n: usize, dt: f64) -> Vec<f64> {
    let normal = Normal::new(0.0, dt.sqrt()).unwrap();
    let mut path = Vec::with_capacity(n);
    let mut x = 0.0;
    for _ in 0..n {
        x += normal.sample(&mut rand::thread_rng());
        path.push(x);
    }
    path
}
