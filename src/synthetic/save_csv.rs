use pyo3::prelude::*;
use std::fs::File;
use std::io::{Write, BufWriter};

#[pyfunction]
pub fn save_spy_to_csv(path: String, prices: Vec<Vec<f64>>) -> PyResult<()> {
    let file = File::create(path)?;
    let mut writer = BufWriter::new(file);

    writeln!(writer, "day,minute,price")?;
    for (day_idx, day) in prices.iter().enumerate() {
        for (minute_idx, price) in day.iter().enumerate() {
            writeln!(writer, "{},{},{}", day_idx + 1, minute_idx, price)?;
        }
    }
    Ok(())
}
