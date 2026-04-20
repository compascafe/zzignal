use std::sync::{Arc, Mutex};
use tokio::sync::{broadcast, RwLock, mpsc as tokio_mpsc};
use sqlx::PgPool;

use crate::worker::{BookSnapshot, Candle, CandleInterval, CmdMsg, MarketInfo, OpenOrder, RecentFill};

pub struct AppState {
    // Estado en memoria (actualizado por el consumer de AppMsg)
    pub status:          RwLock<String>,
    pub market:          RwLock<Option<MarketInfo>>,
    pub book_up:         RwLock<Option<BookSnapshot>>,
    pub book_down:       RwLock<Option<BookSnapshot>>,
    pub balance:         RwLock<Option<f64>>,
    pub btc_price:       RwLock<Option<f64>>,
    pub btc_open:        RwLock<Option<f64>>,
    pub last_trade_up:   RwLock<Option<f64>>,
    pub last_trade_down: RwLock<Option<f64>>,
    pub open_orders:     RwLock<Vec<OpenOrder>>,
    pub recent_fills:    RwLock<Vec<RecentFill>>,
    pub candles:         RwLock<Vec<Candle>>,

    // Control del intervalo de velas (compartido con worker)
    pub interval_arc:    Arc<Mutex<CandleInterval>>,

    // Comandos → worker
    pub cmd_tx:          tokio_mpsc::UnboundedSender<CmdMsg>,

    // Broadcast → clientes WebSocket
    pub broadcast_tx:    broadcast::Sender<String>,

    // Base de datos (None si DATABASE_URL no está configurada)
    pub db:              Option<PgPool>,
}

impl AppState {
    pub fn new(
        cmd_tx:       tokio_mpsc::UnboundedSender<CmdMsg>,
        broadcast_tx: broadcast::Sender<String>,
        interval_arc: Arc<Mutex<CandleInterval>>,
        db:           Option<PgPool>,
    ) -> Arc<Self> {
        Arc::new(Self {
            status:          RwLock::new("Initializing".into()),
            market:          RwLock::new(None),
            book_up:         RwLock::new(None),
            book_down:       RwLock::new(None),
            balance:         RwLock::new(None),
            btc_price:       RwLock::new(None),
            btc_open:        RwLock::new(None),
            last_trade_up:   RwLock::new(None),
            last_trade_down: RwLock::new(None),
            open_orders:     RwLock::new(vec![]),
            recent_fills:    RwLock::new(vec![]),
            candles:         RwLock::new(vec![]),
            interval_arc,
            cmd_tx,
            broadcast_tx,
            db,
        })
    }
}
