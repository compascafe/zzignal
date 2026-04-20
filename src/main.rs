//! Polymarket BTC 15-min — Backend Service
//!
//! Expone:
//!   REST API  → http://0.0.0.0:8080/api/...
//!   WebSocket → ws://0.0.0.0:8080/ws
//!
//! Variables de entorno requeridas (.env):
//!   POLYMARKET_PRIVATE_KEY, CLOB_API_KEY, CLOB_API_SECRET, CLOB_API_PASSPHRASE
//!   DATABASE_URL=postgres://user:pass@host/db
//!
//! Uso: cargo run [--release]

#![allow(dead_code)]

mod api;
mod credentials;
mod db;
mod state;
mod worker;

use std::sync::{mpsc, Arc, Mutex};

use tokio::sync::{broadcast, mpsc as tokio_mpsc};
use tracing::{error, info, warn, Level};
use tracing_subscriber::FmtSubscriber;
use worker::{AppMsg, CandleInterval, CmdMsg, ConnStatus};

use crate::credentials::ClobCredentials;
use crate::state::AppState;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    if let Err(e) = dotenvy::dotenv() {
        eprintln!("[warn] .env no cargado: {e}");
    }

    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::INFO)
        .with_target(false)
        .compact()
        .finish();
    tracing::subscriber::set_global_default(subscriber)?;

    // Credenciales Polymarket
    let creds = match ClobCredentials::from_env() {
        Ok(c) => { info!("Wallet: {}", c.display_address()); Arc::new(c) }
        Err(e) => { error!("Credenciales no disponibles: {:#}", e); return Ok(()); }
    };

    // PostgreSQL (opcional — si DATABASE_URL no está, el backend corre sin DB)
    let db = match std::env::var("DATABASE_URL") {
        Ok(url) => {
            match sqlx::PgPool::connect(&url).await {
                Ok(pool) => {
                    if let Err(e) = db::run_migrations(&pool).await {
                        tracing::warn!("Migraciones DB fallaron: {e}");
                    } else {
                        info!("PostgreSQL OK — migraciones aplicadas");
                    }
                    Some(pool)
                }
                Err(e) => {
                    tracing::warn!("PostgreSQL no disponible ({e}) — arrancando sin DB");
                    None
                }
            }
        }
        Err(_) => {
            info!("DATABASE_URL no configurada — arrancando sin persistencia");
            None
        }
    };

    // Channels
    let (tx, rx)          = mpsc::channel::<AppMsg>();
    let (cmd_tx, cmd_rx)  = tokio_mpsc::unbounded_channel::<CmdMsg>();
    let (bcast_tx, _)     = broadcast::channel::<String>(512);
    let interval_arc      = Arc::new(Mutex::new(CandleInterval::OneMinute));

    // AppState compartido
    let state = AppState::new(
        cmd_tx,
        bcast_tx.clone(),
        Arc::clone(&interval_arc),
        db,
    );

    // Worker (hilo OS con su propio runtime tokio)
    {
        let tx2           = tx.clone();
        let creds2        = Arc::clone(&creds);
        let interval_arc2 = Arc::clone(&interval_arc);
        let bcast_tx2     = bcast_tx.clone();
        std::thread::Builder::new()
            .name("polymarket-worker".into())
            .spawn(move || {
                tokio::runtime::Builder::new_multi_thread()
                    .enable_all()
                    .build()
                    .expect("tokio runtime worker")
                    .block_on(worker::run(tx2, creds2, cmd_rx, interval_arc2, bcast_tx2));
            })
            .expect("spawn worker");
    }

    // Consumer de AppMsg: actualiza estado + persiste en DB + hace broadcast WS
    {
        let state2 = Arc::clone(&state);
        // Bridge std::mpsc → tokio mpsc para poder usarlo en async
        let (bridge_tx, mut bridge_rx) = tokio_mpsc::unbounded_channel::<AppMsg>();
        std::thread::spawn(move || {
            while let Ok(msg) = rx.recv() {
                let _ = bridge_tx.send(msg);
            }
        });
        tokio::spawn(async move {
            while let Some(msg) = bridge_rx.recv().await {
                // Broadcast JSON a clientes WS
                if let Some(json) = msg.to_json() {
                    let _ = state2.broadcast_tx.send(json);
                }
                // Actualizar estado en memoria y persistir en DB
                update_state(&msg, &state2).await;
            }
        });
    }

    // Servidor axum
    let addr = "0.0.0.0:8080";
    let app  = api::router(Arc::clone(&state));

    info!("============================================");
    info!(" Polymarket BTC 15-min Backend");
    info!(" REST API:  http://{}/api/...", addr);
    info!(" WebSocket: ws://{}/ws", addr);
    info!("============================================");

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

// ─── Consumer de AppMsg ───────────────────────────────────────────────────────

/// Tick counter para muestrear BTC ticks (1 de cada 10 actualizaciones → ~1/s)
static BTC_TICK_COUNTER: std::sync::atomic::AtomicU32 =
    std::sync::atomic::AtomicU32::new(0);

async fn update_state(msg: &AppMsg, state: &AppState) {
    match msg {
        AppMsg::Status(s) => {
            let label = match s {
                ConnStatus::Initializing      => "Initializing".into(),
                ConnStatus::Authenticating    => "Authenticating".into(),
                ConnStatus::FetchingMarkets   => "FetchingMarkets".into(),
                ConnStatus::ConnectingWs      => "ConnectingWs".into(),
                ConnStatus::Live              => "LIVE".into(),
                ConnStatus::MarketFound(m)    => format!("MarketFound: {}", m.title),
                ConnStatus::Reconnecting(n)   => format!("Reconnecting ({})", n),
                ConnStatus::Error(e)          => format!("Error: {}", e),
            };
            *state.status.write().await = label;
            if let ConnStatus::MarketFound(info) = s {
                *state.market.write().await = Some(info.clone());
            }
        }

        AppMsg::BookUp(b)        => { *state.book_up.write().await        = Some(b.clone()); }
        AppMsg::BookDown(b)      => { *state.book_down.write().await       = Some(b.clone()); }
        AppMsg::LastTradeUp(p)   => { *state.last_trade_up.write().await   = Some(*p); }
        AppMsg::LastTradeDown(p) => { *state.last_trade_down.write().await  = Some(*p); }
        AppMsg::Balance(b)       => { *state.balance.write().await          = Some(*b); }
        AppMsg::BtcOpen(p)       => { *state.btc_open.write().await         = Some(*p); }

        AppMsg::BtcPrice(p) => {
            *state.btc_price.write().await = Some(*p);
            let n = BTC_TICK_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if n % 10 == 0 {
                if let Err(e) = db::insert_btc_tick(state.db.as_ref(), *p).await {
                    warn!("DB btc_tick: {e}");
                }
            }
        }

        AppMsg::OpenOrders(o) => { *state.open_orders.write().await = o.clone(); }

        AppMsg::RecentFills(fills) => {
            *state.recent_fills.write().await = fills.clone();
            for fill in fills {
                if let Err(e) = db::insert_fill(state.db.as_ref(), fill).await {
                    warn!("DB insert_fill: {e}");
                }
            }
        }

        AppMsg::Candles { interval, candles } => {
            *state.candles.write().await = candles.clone();
            for c in candles {
                if let Err(e) = db::upsert_candle(state.db.as_ref(), interval, c).await {
                    warn!("DB upsert_candle: {e}");
                }
            }
        }

        AppMsg::CandleUpdate(c) => {
            let interval = state.interval_arc
                .lock()
                .map(|g| g.binance_str().to_string())
                .unwrap_or_else(|_| "1m".into());
            let mut candles = state.candles.write().await;
            match candles.last_mut() {
                Some(last) if last.open_time == c.open_time => *last = c.clone(),
                _ => candles.push(c.clone()),
            }
            drop(candles);
            if let Err(e) = db::upsert_candle(state.db.as_ref(), &interval, c).await {
                warn!("DB upsert_candle update: {e}");
            }
        }

        AppMsg::OrderResult(r) => { info!("Order result: {}", r); }
    }
}
