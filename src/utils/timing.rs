use std::time::Instant;

pub fn time_it<F: FnOnce() -> R, R>(f: F) -> (R, f64) {
    let start = Instant::now();
    let result = f();
    let elapsed = start.elapsed().as_secs_f64();
    (result, elapsed)
}
