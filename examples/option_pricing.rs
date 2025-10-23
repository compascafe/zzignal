use zzignal::options::payoff;

fn main() {
    let payoff_val = payoff::call_payoff(105.0, 100.0);
    println!("Call option payoff: {}", payoff_val);
}
