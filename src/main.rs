//! Polymarket BTC 15-min — Backend Service
//! Ejecuta el worker asíncrono con WebSocket server para clientes JavaFX.
//!
//! Uso: cargo run --release
//!
//! Variables de entorno requeridas en .env:
//! - POLYMARKET_PRIVATE_KEY=0x...   (clave privada Ethereum)
//! - CLOB_API_KEY=...               (UUID de API)
//! - CLOB_API_SECRET=...             (secret)
//! - CLOB_API_PASSPHRASE=...         (passphrase)

#![allow(dead_code)]

mod credentials;
mod worker;

use std::sync::{mpsc, Arc, Mutex};

use tokio::sync::mpsc as tokio_mpsc;
use tracing::{error, info, Level};
use tracing_subscriber::FmtSubscriber;
use worker::{AppMsg, CandleInterval, CmdMsg, ConnStatus};
use crate::credentials::ClobCredentials;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    if let Err(e) = dotenvy::dotenv() {
        eprintln!("[warn] .env no cargado: {e}");
    }

    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::INFO)
        .with_target(false)
        .with_thread_ids(false)
        .with_file(false)
        .with_line_number(false)
        .compact()
        .finish();
    tracing::subscriber::set_global_default(subscriber)?;

    let creds = match ClobCredentials::from_env() {
        Ok(c) => {
            info!("Wallet: {}", c.display_address());
            Arc::new(c)
        }
        Err(e) => {
            error!("Credenciales no disponibles: {:#}", e);
            return Ok(());
        }
    };

    let (tx, rx) = mpsc::channel::<AppMsg>();
    let (cmd_tx, cmd_rx) = tokio_mpsc::unbounded_channel::<CmdMsg>();
    let interval_arc = Arc::new(Mutex::new(CandleInterval::OneSecond));

    let tx_clone = tx.clone();
    std::thread::Builder::new()
        .name("tokio-worker".into())
        .spawn(move || {
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("tokio runtime")
                .block_on(worker::run(tx_clone, creds, cmd_rx, interval_arc));
        })
        .expect("spawn worker");

    let _ = cmd_tx;

    info!("===========================================");
    info!(" Polymarket BTC 15-min Backend Service");
    info!("===========================================");
    info!("WebSocket server: ws://localhost:8080");
    info!("===========================================");

    while let Ok(msg) = rx.recv() {
        match &msg {
            AppMsg::Status(s) => {
                let label = match s {
                    ConnStatus::Live => "LIVE",
                    ConnStatus::Reconnecting(n) => { eprintln!("Reconnecting (attempt {})...", n); continue; }
                    ConnStatus::Error(e) => { eprintln!("Error: {}", e); continue; }
                    _ => continue,
                };
                info!("Status: {}", label);
            }
            AppMsg::BtcPrice(p) => {
                info!("BTC: ${:.2}", p);
            }
            AppMsg::Balance(b) => {
                info!("Balance: ${:.2} USDC", b);
            }
            AppMsg::OrderResult(r) => {
                info!("Order: {}", r);
            }
            _ => {}
        }
    }

    Ok(())
}
