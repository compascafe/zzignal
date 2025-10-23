pub fn call_payoff(spot: f64, strike: f64) -> f64 {
    (spot - strike).max(0.0)
}
