use std::time::Instant;

pub struct Timer {
    start: Instant,
}

impl Timer {
    pub fn start() -> Self {
        Timer { start: Instant::now() }
    }
    pub fn stop(&mut self) -> f64 {
        self.start.elapsed().as_secs_f64()
    }
}
