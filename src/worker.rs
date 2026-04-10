/// Background worker — corre dentro del runtime de Tokio en un hilo separado.
///
/// Mercado BTC 15-min tiene DOS tokens: UP y DOWN.
/// Se suscribe al WebSocket de ambos simultáneamente.
/// El precio de BTC en tiempo real llega por Chainlink via RTDS.
use std::net::SocketAddr;
use std::sync::{mpsc, Arc};
use std::time::Duration;

use alloy::signers::local::PrivateKeySigner;
use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Timelike, Utc};
use futures_util::{SinkExt, StreamExt};
use polymarket_client_sdk::auth::{Credentials, Normal, Uuid};
use polymarket_client_sdk::auth::state::Authenticated;
use polymarket_client_sdk::clob::types::request::{
    BalanceAllowanceRequest, CancelMarketOrderRequest, LastTradePriceRequest,
    OrderBookSummaryRequest, OrdersRequest, TradesRequest,
};
use polymarket_client_sdk::clob::types::Side as ClobSideType;
use polymarket_client_sdk::clob::types::{Amount, OrderType, Side as ClobSide, SignatureType};
use polymarket_client_sdk::clob::{Client, Config};
use polymarket_client_sdk::types::Address;
use polymarket_client_sdk::gamma;
use polymarket_client_sdk::gamma::types::request::{EventBySlugRequest, MarketsRequest, PublicProfileRequest};
use polymarket_client_sdk::types::{Decimal, U256};
use reqwest::Client as HttpClient;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, mpsc as tokio_mpsc};
use tokio_tungstenite::{accept_async, connect_async, tungstenite::Message};
use tracing::{error, info, warn};

use crate::credentials::ClobCredentials;

// ─── Tipos públicos ───────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct MarketInfo {
    pub title:          String,
    pub token_id_up:    String,          // índice 0: Up / Yes
    pub token_id_down:  Option<String>,  // índice 1: Down / No
    pub outcome_up:     String,          // "Up" / "Yes"
    pub outcome_down:   String,          // "Down" / "No"
    pub end_date:       DateTime<Utc>,
    pub active:         bool,
    /// Precio BTC al inicio del intervalo — proviene del campo groupItemThreshold
    /// del mercado en la Gamma API (el mismo valor que muestra Polymarket).
    pub price_to_beat:  Option<f64>,
}

#[derive(Debug, Clone)]
pub struct PriceLevel {
    pub price: f64,
    pub size:  f64,
}

#[derive(Debug, Clone)]
pub struct BookSnapshot {
    pub bids: Vec<PriceLevel>,
    pub asks: Vec<PriceLevel>,
}

#[derive(Debug, Clone)]
pub enum ConnStatus {
    Initializing,
    Authenticating,
    FetchingMarkets,
    MarketFound(MarketInfo),
    ConnectingWs,
    Live,
    Reconnecting(#[allow(dead_code)] u32),
    #[allow(dead_code)]
    Error(String),
}

/// Orden abierta en el CLOB
#[derive(Debug, Clone)]
pub struct OpenOrder {
    pub id:           String,
    pub outcome:      String,   // "Up" / "Down"
    pub side:         OrderSide,
    pub price:        f64,
    pub size_orig:    f64,      // tamaño original
    pub size_matched: f64,      // ejecutado
}

/// Trade/fill reciente
#[derive(Debug, Clone)]
pub struct RecentFill {
    pub outcome: String,
    pub side:    OrderSide,
    pub price:   f64,
    pub size:    f64,
    /// Hora exacta del match, e.g. "14:32:07"
    pub time:    String,
    /// Sesión 15-min de Polymarket, e.g. "14:30"
    pub session: String,
}

/// Vela OHLCV de BTC/USDT (Binance 1m)
#[derive(Debug, Clone)]
pub struct Candle {
    pub open_time: i64,  // Unix ms
    pub open:      f64,
    pub high:      f64,
    pub low:       f64,
    pub close:     f64,
    pub volume:    f64,
}

#[derive(Debug)]
pub enum AppMsg {
    Status(ConnStatus),
    BookUp(BookSnapshot),
    BookDown(BookSnapshot),
    LastTradeUp(f64),
    LastTradeDown(f64),
    Balance(f64),
    BtcOpen(f64),
    BtcPrice(f64),
    OrderResult(String),
    OpenOrders(Vec<OpenOrder>),
    RecentFills(Vec<RecentFill>),
    Candles(Vec<Candle>),     // batch inicial / cambio de intervalo
    CandleUpdate(Candle),     // actualización de la última vela (WS)
}

// ─── Comandos UI → worker ────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OrderSide { Buy, Sell }

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Outcome { Up, Down }

#[derive(Debug)]
pub enum CmdMsg {
    PlaceLimitOrder  { side: OrderSide, outcome: Outcome, price: f64, size: f64 },
    PlaceMarketOrder { side: OrderSide, outcome: Outcome, amount_usdc: f64 },
    /// Scalp: compra a `price`, espera fill, luego vende a `target_price`.
    ScalpBuy { outcome: Outcome, price: f64, size: f64, target_price: f64 },
    CancelOrder      { order_id: String },
    CancelMarket,
}

const CLOB_WS: &str = "wss://ws-subscriptions-clob.polymarket.com/ws/market";

// ─── Intervalo de velas ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CandleInterval {
    OneSecond,
    OneMinute,
    FiveMinutes,
    FifteenMinutes,
    OneHour,
}

impl CandleInterval {
    pub fn binance_str(self) -> &'static str {
        match self {
            Self::OneSecond      => "1s",
            Self::OneMinute      => "1m",
            Self::FiveMinutes    => "5m",
            Self::FifteenMinutes => "15m",
            Self::OneHour        => "1h",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::OneSecond      => "1s",
            Self::OneMinute      => "1m",
            Self::FiveMinutes    => "5m",
            Self::FifteenMinutes => "15m",
            Self::OneHour        => "1h",
        }
    }
    /// Segundos entre refrescos REST
    pub fn refresh_secs(self) -> u64 {
        match self {
            Self::OneSecond      => 3,
            Self::OneMinute      => 15,
            Self::FiveMinutes    => 30,
            Self::FifteenMinutes => 60,
            Self::OneHour        => 120,
        }
    }
    /// Número de velas a pedir
    pub fn limit(self) -> u32 {
        match self {
            Self::OneSecond      => 300,
            Self::OneMinute      => 200,
            Self::FiveMinutes    => 200,
            Self::FifteenMinutes => 200,
            Self::OneHour        => 100,
        }
    }
}

// ─── Punto de entrada ─────────────────────────────────────────────────────────

pub async fn run(
    tx:           mpsc::Sender<AppMsg>,
    creds:        Arc<ClobCredentials>,
    mut cmd_rx:   tokio_mpsc::UnboundedReceiver<CmdMsg>,
    interval_arc: Arc<std::sync::Mutex<CandleInterval>>,
) {
    let _ = tx.send(AppMsg::Status(ConnStatus::Initializing));

    let (broadcast_tx, _) = broadcast::channel::<String>(100);

    let ws_broadcast = broadcast_tx.clone();
    tokio::spawn(async move {
        if let Err(e) = run_websocket_server("0.0.0.0:8080", ws_broadcast).await {
            error!("WebSocket server error: {}", e);
        }
    });

    let mut attempts: u32 = 0;
    loop {
        match run_cycle(&tx, &creds, &mut cmd_rx, Arc::clone(&interval_arc), broadcast_tx.clone()).await {
            Ok(_) => {
                attempts = 0;
                info!("Ciclo completado, reiniciando...");
            }
            Err(e) => {
                attempts += 1;
                error!("Error en worker (intento {}): {:#}", attempts, e);
                let _ = tx.send(AppMsg::Status(ConnStatus::Reconnecting(attempts)));
                tokio::time::sleep(Duration::from_secs((5 * attempts).min(30) as u64)).await;
            }
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

// ─── Ciclo principal ──────────────────────────────────────────────────────────

async fn run_cycle(
    tx:           &mpsc::Sender<AppMsg>,
    creds:        &ClobCredentials,
    cmd_rx:       &mut tokio_mpsc::UnboundedReceiver<CmdMsg>,
    interval_arc: Arc<std::sync::Mutex<CandleInterval>>,
    broadcast_tx: broadcast::Sender<String>,
) -> Result<()> {
    let _ = tx.send(AppMsg::Status(ConnStatus::Authenticating));
    let signer = creds.build_signer()?;

    // 1. Obtener proxy wallet desde Gamma (no requiere auth)
    let gamma_client = gamma::Client::default();
    let proxy_addr: Option<Address> = {
        let eoa: Address = creds.wallet_address.parse().ok().unwrap_or_default();
        let req = PublicProfileRequest::builder().address(eoa).build();
        match gamma_client.public_profile(&req).await {
            Ok(p) => {
                if let Some(a) = p.proxy_wallet {
                    info!("Proxy wallet: {:#x}", a);
                    Some(a)
                } else {
                    info!("Sin proxy_wallet en perfil, usando EOA");
                    None
                }
            }
            Err(e) => { warn!("public_profile falló: {e}"); None }
        }
    };

    // 2. Autenticar CLOB.
    //    Si hay proxy wallet → SignatureType::Proxy + funder = proxy_addr
    //    El CLOB buscará el saldo USDC bajo esa dirección (donde está el dinero real).
    let api_key: Uuid = creds.api_key.parse().context("CLOB_API_KEY no es UUID")?;
    let secret_padded = {
        let s = &creds.api_secret;
        let pad = (4 - s.len() % 4) % 4;
        format!("{}{}", s, "=".repeat(pad))
    };
    let l2_creds = Credentials::new(api_key, secret_padded, creds.api_passphrase.clone());

    let base_builder = Client::new("https://clob.polymarket.com", Config::default())
        .context("Error creando cliente CLOB")?
        .authentication_builder(&signer)
        .credentials(l2_creds);

    let clob_client = if let Some(proxy) = proxy_addr {
        base_builder
            .funder(proxy)
            .signature_type(SignatureType::Proxy)
            .authenticate().await
            .context("Fallo de autenticación (Proxy)")?
    } else {
        base_builder.authenticate().await
            .context("Fallo de autenticación (EOA)")?
    };

    // 3. Balance USDC via CLOB (ahora con el funder correcto devuelve el saldo real)
    fetch_and_send_balance(&clob_client, tx).await;

    // 4. Descubrir mercado
    let _ = tx.send(AppMsg::Status(ConnStatus::FetchingMarkets));
    let info = discover_btc_market(&gamma_client).await?;

    let token_up:   U256 = info.token_id_up.parse().context("token_up parse")?;
    let token_down: Option<U256> = info
        .token_id_down
        .as_deref()
        .and_then(|s| s.parse().ok());

    let _ = tx.send(AppMsg::Status(ConnStatus::MarketFound(info.clone())));

    // 4a. Precio a batir: leído directamente de groupItemThreshold en Gamma API.
    //     Si no viene, fallback a Pyth Network.
    if let Some(p) = info.price_to_beat {
        info!("Precio a batir (Gamma groupItemThreshold): {}", p);
        let _ = tx.send(AppMsg::BtcOpen(p));
    } else if let Some(p) = fetch_btc_open_price(&info.end_date).await {
        info!("Precio a batir (Pyth fallback): {}", p);
        let _ = tx.send(AppMsg::BtcOpen(p));
    }

    // 4b. Snapshots REST iniciales (ambos tokens) — timeout 10s para evitar cuelgues
    const REST_TIMEOUT: Duration = Duration::from_secs(10);

    match tokio::time::timeout(REST_TIMEOUT, clob_client
        .order_book(&OrderBookSummaryRequest::builder().token_id(token_up).build()))
        .await
    {
        Ok(Ok(snap)) => { let _ = tx.send(AppMsg::BookUp(convert_book(&snap))); }
        Ok(Err(e))   => warn!("order_book UP falló: {}", e),
        Err(_)       => warn!("order_book UP timeout"),
    }

    if let Some(td) = token_down {
        match tokio::time::timeout(REST_TIMEOUT, clob_client
            .order_book(&OrderBookSummaryRequest::builder().token_id(td).build()))
            .await
        {
            Ok(Ok(snap)) => { let _ = tx.send(AppMsg::BookDown(convert_book(&snap))); }
            Ok(Err(e))   => warn!("order_book DOWN falló: {}", e),
            Err(_)       => warn!("order_book DOWN timeout"),
        }
    }

    // 4c. Last trade price REST inicial
    for (tid, is_up) in [(token_up, true), (token_down.unwrap_or(token_up), false)] {
        if !is_up && token_down.is_none() { break; }
        let req = LastTradePriceRequest::builder().token_id(tid).build();
        match tokio::time::timeout(REST_TIMEOUT, clob_client.last_trade_price(&req)).await {
            Ok(Ok(r)) => {
                let p: f64 = r.price.to_string().parse().unwrap_or(0.0);
                if p > 0.0 {
                    if is_up { let _ = tx.send(AppMsg::LastTradeUp(p));   }
                    else      { let _ = tx.send(AppMsg::LastTradeDown(p)); }
                }
            }
            Ok(Err(e)) => warn!("last_trade_price falló (up={}): {}", is_up, e),
            Err(_)     => warn!("last_trade_price timeout (up={})", is_up),
        }
    }

    // 5. Órdenes abiertas y fills iniciales
    fetch_and_send_orders(&clob_client, tx, token_up, token_down).await;

    // 6. Precio BTC en tiempo real (Binance aggTrade WS) en segundo plano
    {
        let tx2 = tx.clone();
        tokio::spawn(async move { run_btc_price_stream(tx2).await });
    }

    // 6b. Velas BTC/USDT — fetch inicial + refresco adaptativo según intervalo
    {
        let tx3 = tx.clone();
        let iv  = Arc::clone(&interval_arc);
        let bt3 = broadcast_tx.clone();
        tokio::spawn(async move { run_candle_stream(tx3, iv, bt3).await });
    }

    // 7. WebSocket + comandos
    let _ = tx.send(AppMsg::Status(ConnStatus::ConnectingWs));
    run_live(token_up, token_down, tx.clone(), clob_client, signer, cmd_rx, broadcast_tx).await
}

// ─── Loop WS + comandos ───────────────────────────────────────────────────────

async fn run_live(
    token_up:   U256,
    token_down: Option<U256>,
    tx:         mpsc::Sender<AppMsg>,
    client:     Client<Authenticated<Normal>>,
    signer:     PrivateKeySigner,
    cmd_rx:     &mut tokio_mpsc::UnboundedReceiver<CmdMsg>,
    broadcast_tx: broadcast::Sender<String>,
) -> Result<()> {
    let (ws_stream, _) = connect_async(CLOB_WS)
        .await
        .map_err(|e| anyhow!("WS connect: {}", e))?;
    let (mut write, mut read) = ws_stream.split();

    // Suscribir ambos tokens
    let mut asset_ids = vec![token_up.to_string()];
    if let Some(td) = token_down { asset_ids.push(td.to_string()); }
    let sub = serde_json::json!({ "assets_ids": asset_ids, "type": "market" });
    write.send(sub.to_string().into()).await
        .map_err(|e| anyhow!("WS subscribe: {}", e))?;

    let _ = tx.send(AppMsg::Status(ConnStatus::Live));
    info!("WebSocket LIVE — UP:{} DOWN:{:?}", token_up, token_down);

    let up_str   = token_up.to_string();
    let down_str = token_down.map(|t| t.to_string());

    let broadcast_tx_ws = broadcast_tx.clone();

    // Timer balance + órdenes cada 5s
    let mut bal_timer = tokio::time::interval(Duration::from_secs(5));
    bal_timer.tick().await;

    loop {
        tokio::select! {
            // ── WebSocket ─────────────────────────────────────────────────────
            msg = read.next() => {
                match msg {
                    Some(Ok(m)) if m.is_text() => {
                        if let Ok(text) = m.into_text() {
                            handle_ws_text(&text, &tx, &up_str, down_str.as_deref(), &broadcast_tx_ws);
                        }
                    }
                    Some(Ok(m)) if m.is_ping() => {
                        let _ = write.send(Message::Pong(m.into_data())).await;
                    }
                    Some(Ok(m)) if m.is_close() => {
                        warn!("WS cerrado por servidor");
                        break;
                    }
                    Some(Ok(_)) => {}
                    Some(Err(e)) => return Err(anyhow!("WS error: {}", e)),
                    None => break,
                }
            }

            // ── Comandos UI ───────────────────────────────────────────────────
            cmd = cmd_rx.recv() => {
                match cmd {
                    Some(CmdMsg::PlaceLimitOrder { side, outcome, price, size }) => {
                        let tid = pick_token(outcome, token_up, token_down);
                        let c2 = client.clone(); let tx2 = tx.clone(); let s2 = signer.clone();
                        let bt2 = broadcast_tx.clone();
                        tokio::spawn(async move {
                            handle_limit_order(&c2, &s2, &tx2, &bt2, tid, side, price, size).await;
                        });
                    }
                    Some(CmdMsg::PlaceMarketOrder { side, outcome, amount_usdc }) => {
                        let tid = pick_token(outcome, token_up, token_down);
                        let c2 = client.clone(); let tx2 = tx.clone(); let s2 = signer.clone();
                        let bt2 = broadcast_tx.clone();
                        tokio::spawn(async move {
                            handle_market_order(&c2, &s2, &tx2, &bt2, tid, side, amount_usdc).await;
                        });
                    }
                    Some(CmdMsg::ScalpBuy { outcome, price, size, target_price }) => {
                        let tid = pick_token(outcome, token_up, token_down);
                        let c2 = client.clone(); let tx2 = tx.clone(); let s2 = signer.clone();
                        let bt2 = broadcast_tx.clone();
                        tokio::spawn(async move {
                            handle_scalp_buy(&c2, &s2, &tx2, &bt2, tid, price, size, target_price).await;
                        });
                    }
                    Some(CmdMsg::CancelOrder { order_id }) => {
                        let c2 = client.clone(); let tx2 = tx.clone();
                        let bt2 = broadcast_tx.clone();
                        tokio::spawn(async move {
                            handle_cancel_order(&c2, &tx2, &bt2, &order_id).await;
                        });
                    }
                    Some(CmdMsg::CancelMarket) => {
                        let c2 = client.clone(); let tx2 = tx.clone();
                        let bt2 = broadcast_tx.clone();
                        tokio::spawn(async move {
                            handle_cancel_market(&c2, &tx2, &bt2, token_up, token_down).await;
                        });
                    }
                    None => break,
                }
            }

            // ── Balance + órdenes cada 5s — spawneado para no bloquear el WS ─
            _ = bal_timer.tick() => {
                let c2 = client.clone(); let tx2 = tx.clone();
                tokio::spawn(async move {
                    fetch_and_send_balance(&c2, &tx2).await;
                    fetch_and_send_orders(&c2, &tx2, token_up, token_down).await;
                });
            }
        }
    }
    Ok(())
}

// ─── BTC/USD precio en tiempo real — Binance aggTrade WebSocket ───────────────
//
// Usa wss://stream.binance.com:9443/ws/btcusdt@aggTrade (público, sin auth).
// Actualiza ~cada segundo, mucho más frecuente que Chainlink para intervalos 15-min.
// Se reconecta automáticamente en caso de error.

const BINANCE_WS: &str = "wss://stream.binance.com:9443/ws/btcusdt@aggTrade";

async fn run_btc_price_stream(tx: mpsc::Sender<AppMsg>) {
    let mut backoff = Duration::from_secs(2);
    loop {
        match connect_async(BINANCE_WS).await {
            Ok((ws_stream, _)) => {
                info!("Binance BTC/USD stream conectado");
                backoff = Duration::from_secs(2);
                let (_, mut read) = ws_stream.split();
                while let Some(msg) = read.next().await {
                    match msg {
                        Ok(m) if m.is_text() => {
                            if let Ok(text) = m.into_text() {
                                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
                                    // aggTrade: campo "p" = precio del trade
                                    if let Some(p) = json.get("p")
                                        .and_then(|v| v.as_str())
                                        .and_then(|s| s.parse::<f64>().ok())
                                        .filter(|&p| p > 0.0)
                                    {
                                        let _ = tx.send(AppMsg::BtcPrice(p));
                                    }
                                }
                            }
                        }
                        Ok(m) if m.is_close() => break,
                        Ok(_) => {}
                        Err(e) => { warn!("Binance WS error: {}", e); break; }
                    }
                }
                warn!("Binance BTC/USD stream desconectado, reconectando...");
            }
            Err(e) => {
                warn!("Binance WS connect falló: {} — reintento en {}s", e, backoff.as_secs());
            }
        }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(Duration::from_secs(30));
    }
}

// ─── Velas BTC/USDT — Binance REST klines (1m, últimas 200) ──────────────────

/// Stream de velas en tiempo real: REST histórico + WebSocket kline de Binance.
/// Actualiza la última vela en tiempo real (AppMsg::CandleUpdate) sin re-enviar todo el array.
async fn run_candle_stream(
    tx:           mpsc::Sender<AppMsg>,
    interval_arc: Arc<std::sync::Mutex<CandleInterval>>,
    broadcast_tx: broadcast::Sender<String>,
) {
    let mut current_iv = interval_arc.lock().map(|g| *g).unwrap_or(CandleInterval::OneSecond);

    loop {
        let iv = interval_arc.lock().map(|g| *g).unwrap_or(current_iv);
        if iv != current_iv {
            current_iv = iv;
            let _ = tx.send(AppMsg::Candles(vec![])); // limpiar UI
        }

        // 1. Histórico via REST
        if let Some(candles) = fetch_binance_klines(current_iv.binance_str(), current_iv.limit()).await {
            let _ = tx.send(AppMsg::Candles(candles.clone()));
            if let Some(json) = AppMsg::Candles(candles).to_json() {
                let _ = broadcast_tx.send(json);
            }
        }

        // 2. Tiempo real via WebSocket kline
        let ws_url = format!(
            "wss://stream.binance.com:9443/ws/btcusdt@kline_{}",
            current_iv.binance_str()
        );
        match connect_async(&ws_url).await {
            Ok((ws_stream, _)) => {
                info!("Kline WS: {ws_url}");
                let (_, mut read) = ws_stream.split();
                loop {
                    // Detectar cambio de intervalo
                    if interval_arc.lock().map(|g| *g).unwrap_or(current_iv) != current_iv {
                        break;
                    }
                    match tokio::time::timeout(Duration::from_secs(30), read.next()).await {
                        Ok(Some(Ok(msg))) if msg.is_text() => {
                            if let Ok(text) = msg.into_text() {
                                if let Some(c) = parse_kline_ws(&text) {
                                    let _ = tx.send(AppMsg::CandleUpdate(c.clone()));
                                    if let Some(json) = AppMsg::CandleUpdate(c).to_json() {
                                        let _ = broadcast_tx.send(json);
                                    }
                                }
                            }
                        }
                        Ok(Some(Err(e))) => { warn!("Kline WS err: {e}"); break; }
                        Ok(None) | Err(_) => { warn!("Kline WS desconectado"); break; }
                        Ok(Some(Ok(_))) => {}
                    }
                }
            }
            Err(e) => {
                warn!("Kline WS connect falló: {e} — reintento en 3s");
                tokio::time::sleep(Duration::from_secs(3)).await;
            }
        }
    }
}

fn parse_kline_ws(text: &str) -> Option<Candle> {
    let json: serde_json::Value = serde_json::from_str(text).ok()?;
    let k = json.get("k")?;
    Some(Candle {
        open_time: k.get("t")?.as_i64()?,
        open:   k.get("o")?.as_str()?.parse().ok()?,
        high:   k.get("h")?.as_str()?.parse().ok()?,
        low:    k.get("l")?.as_str()?.parse().ok()?,
        close:  k.get("c")?.as_str()?.parse().ok()?,
        volume: k.get("v")?.as_str()?.parse().ok()?,
    })
}

async fn fetch_binance_klines(interval: &str, limit: u32) -> Option<Vec<Candle>> {
    let url = format!(
        "https://api.binance.com/api/v3/klines?symbol=BTCUSDT&interval={interval}&limit={limit}"
    );
    let resp = HttpClient::new().get(&url).timeout(Duration::from_secs(10)).send().await.ok()?;
    let raw: serde_json::Value = resp.json().await.ok()?;
    let arr = raw.as_array()?;
    Some(arr.iter().filter_map(|k| {
        let a = k.as_array()?;
        Some(Candle {
            open_time: a.get(0)?.as_i64()?,
            open:   a.get(1)?.as_str()?.parse().ok()?,
            high:   a.get(2)?.as_str()?.parse().ok()?,
            low:    a.get(3)?.as_str()?.parse().ok()?,
            close:  a.get(4)?.as_str()?.parse().ok()?,
            volume: a.get(5)?.as_str()?.parse().ok()?,
        })
    }).collect())
}

// ─── Operaciones de trading ───────────────────────────────────────────────────

fn pick_token(outcome: Outcome, up: U256, down: Option<U256>) -> U256 {
    match outcome {
        Outcome::Up   => up,
        Outcome::Down => down.unwrap_or(up),
    }
}

async fn handle_limit_order(
    client:       &Client<Authenticated<Normal>>,
    signer:       &PrivateKeySigner,
    tx:           &mpsc::Sender<AppMsg>,
    broadcast_tx: &broadcast::Sender<String>,
    token_id:     U256,
    side:         OrderSide,
    price:        f64,
    size:         f64,
) {
    let price_dec: Decimal = match format!("{:.2}", price).parse() {
        Ok(d) => d,
        Err(_) => { 
            let msg = "Precio inválido".to_string();
            let _ = tx.send(AppMsg::OrderResult(msg.clone()));
            if let Some(json) = AppMsg::OrderResult(msg).to_json() { let _ = broadcast_tx.send(json); }
            return; 
        }
    };
    let size_dec: Decimal = match format!("{:.2}", size).parse() {
        Ok(d) => d,
        Err(_) => { 
            let msg = "Tamaño inválido".to_string();
            let _ = tx.send(AppMsg::OrderResult(msg.clone()));
            if let Some(json) = AppMsg::OrderResult(msg).to_json() { let _ = broadcast_tx.send(json); }
            return; 
        }
    };
    let clob_side = if matches!(side, OrderSide::Buy) { ClobSide::Buy } else { ClobSide::Sell };

    let result: Result<_> = async {
        let order = client.limit_order()
            .token_id(token_id)
            .order_type(OrderType::GTC)
            .price(price_dec)
            .size(size_dec)
            .side(clob_side)
            .build().await?;
        let signed = client.sign(signer, order).await?;
        client.post_order(signed).await.map_err(|e| anyhow!(e))
    }.await;

    let side_str = if matches!(side, OrderSide::Buy) { "BUY" } else { "SELL" };
    let msg = match result {
        Ok(r) if r.success => format!("✓ Limit {}: #{}", side_str, &r.order_id[..r.order_id.len().min(16)]),
        Ok(r)              => format!("✗ Rechazada: {}", r.error_msg.unwrap_or_default()),
        Err(e)             => format!("✗ Error: {}", e),
    };
    let _ = tx.send(AppMsg::OrderResult(msg.clone()));
    if let Some(json) = AppMsg::OrderResult(msg).to_json() { let _ = broadcast_tx.send(json); }
}

async fn handle_market_order(
    client:       &Client<Authenticated<Normal>>,
    signer:       &PrivateKeySigner,
    tx:           &mpsc::Sender<AppMsg>,
    broadcast_tx: &broadcast::Sender<String>,
    token_id:     U256,
    side:         OrderSide,
    amount_usdc:  f64,
) {
    let amount_dec: Decimal = match format!("{:.2}", amount_usdc).parse() {
        Ok(d) => d,
        Err(_) => { 
            let msg = "Monto inválido".to_string();
            let _ = tx.send(AppMsg::OrderResult(msg.clone()));
            if let Some(json) = AppMsg::OrderResult(msg).to_json() { let _ = broadcast_tx.send(json); }
            return; 
        }
    };
    let clob_side = if matches!(side, OrderSide::Buy) { ClobSide::Buy } else { ClobSide::Sell };

    let result: Result<_> = async {
        let amount = Amount::usdc(amount_dec).map_err(|e| anyhow!(e))?;
        let order = client.market_order()
            .token_id(token_id)
            .amount(amount)
            .side(clob_side)
            .build().await?;
        let signed = client.sign(signer, order).await?;
        client.post_order(signed).await.map_err(|e| anyhow!(e))
    }.await;

    let side_str = if matches!(side, OrderSide::Buy) { "BUY" } else { "SELL" };
    let msg = match result {
        Ok(r) if r.success => format!("✓ Market {}: #{}", side_str, &r.order_id[..r.order_id.len().min(16)]),
        Ok(r)              => format!("✗ Rechazada: {}", r.error_msg.unwrap_or_default()),
        Err(e)             => format!("✗ Error: {}", e),
    };
    let _ = tx.send(AppMsg::OrderResult(msg.clone()));
    if let Some(json) = AppMsg::OrderResult(msg).to_json() { let _ = broadcast_tx.send(json); }
}

async fn handle_cancel_market(
    client:       &Client<Authenticated<Normal>>,
    tx:           &mpsc::Sender<AppMsg>,
    broadcast_tx: &broadcast::Sender<String>,
    token_up:     U256,
    token_down:   Option<U256>,
) {
    let req_up = CancelMarketOrderRequest::builder().asset_id(token_up).build();
    let up_result = client.cancel_market_orders(&req_up).await;

    let down_result = if let Some(td) = token_down {
        let req_dn = CancelMarketOrderRequest::builder().asset_id(td).build();
        Some(client.cancel_market_orders(&req_dn).await)
    } else {
        None
    };

    let msg = match (up_result, down_result) {
        (Ok(u), Some(Ok(d))) => format!("✓ Canceladas: {} UP + {} DOWN", u.canceled.len(), d.canceled.len()),
        (Ok(u), None)        => format!("✓ Canceladas: {} órdenes UP", u.canceled.len()),
        (Err(e), _)          => format!("✗ Cancel error: {}", e),
        (_, Some(Err(e)))    => format!("✗ Cancel DOWN error: {}", e),
    };
    let _ = tx.send(AppMsg::OrderResult(msg.clone()));
    if let Some(json) = AppMsg::OrderResult(msg).to_json() { let _ = broadcast_tx.send(json); }
}

async fn handle_cancel_order(
    client:       &Client<Authenticated<Normal>>,
    tx:           &mpsc::Sender<AppMsg>,
    broadcast_tx: &broadcast::Sender<String>,
    order_id:     &str,
) {
    match client.cancel_order(order_id).await {
        Ok(r) => {
            let n = r.canceled.len();
            let msg = if n > 0 {
                format!("✓ Orden #{} cancelada", &order_id[..order_id.len().min(12)])
            } else {
                let reason = r.not_canceled.values().next().cloned().unwrap_or_default();
                format!("✗ No cancelada: {reason}")
            };
            let _ = tx.send(AppMsg::OrderResult(msg.clone()));
            if let Some(json) = AppMsg::OrderResult(msg).to_json() { let _ = broadcast_tx.send(json); }
        }
        Err(e) => {
            let msg = format!("✗ Cancel error: {e}");
            let _ = tx.send(AppMsg::OrderResult(msg.clone()));
            if let Some(json) = AppMsg::OrderResult(msg).to_json() { let _ = broadcast_tx.send(json); }
        }
    }
}

async fn fetch_and_send_balance(
    client: &Client<Authenticated<Normal>>,
    tx:     &mpsc::Sender<AppMsg>,
) {
    if let Err(e) = client.update_balance_allowance(BalanceAllowanceRequest::default()).await {
        error!("update_balance_allowance: {:#}", e);
    }
    match client.balance_allowance(BalanceAllowanceRequest::default()).await {
        Ok(b) => {
            let raw: f64 = b.balance.to_string().parse().unwrap_or(0.0);
            // El CLOB devuelve USDC en unidades raw con 6 decimales (ej: 35168666 = $35.17)
            let bal = if raw > 1_000.0 { raw / 1_000_000.0 } else { raw };
            info!("Balance USDC raw={} → ${:.2}", raw, bal);
            let _ = tx.send(AppMsg::Balance(bal));
        }
        Err(e) => error!("balance_allowance: {:#}", e),
    }
}


/// Consulta las órdenes abiertas y fills recientes del CLOB y los envía a la UI.
async fn fetch_and_send_orders(
    client:     &Client<Authenticated<Normal>>,
    tx:         &mpsc::Sender<AppMsg>,
    token_up:   U256,
    token_down: Option<U256>,
) {
    const TIMEOUT: Duration = Duration::from_secs(10);

    // Órdenes abiertas (todos los tokens del usuario; filtramos por nuestros tokens)
    let orders_req = OrdersRequest::builder().build();
    match tokio::time::timeout(TIMEOUT, client.orders(&orders_req, None)).await {
        Ok(Ok(list)) => {
            let up_str   = token_up.to_string();
            let down_str = token_down.map(|t| t.to_string());

            let open: Vec<OpenOrder> = list.data.iter().map(|o| {
                let is_up = o.asset_id.to_string() == up_str;
                let outcome = if is_up {
                    "Up".to_string()
                } else if down_str.as_deref().map_or(false, |d| o.asset_id.to_string() == d) {
                    "Down".to_string()
                } else {
                    o.outcome.clone()
                };
                let side = if matches!(o.side, ClobSideType::Buy) { OrderSide::Buy } else { OrderSide::Sell };
                OpenOrder {
                    id:           o.id.clone(),
                    outcome,
                    side,
                    price:        o.price.to_string().parse().unwrap_or(0.0),
                    size_orig:    o.original_size.to_string().parse().unwrap_or(0.0),
                    size_matched: o.size_matched.to_string().parse().unwrap_or(0.0),
                }
            }).collect();

            info!("Órdenes abiertas: {}", open.len());
            let _ = tx.send(AppMsg::OpenOrders(open));
        }
        Ok(Err(e)) => warn!("orders: {}", e),
        Err(_)     => warn!("orders timeout"),
    }

    // Fills recientes (últimos 20, ambos tokens)
    let trades_req = TradesRequest::builder().build();
    match tokio::time::timeout(TIMEOUT, client.trades(&trades_req, None)).await {
        Ok(Ok(list)) => {
            let fills: Vec<RecentFill> = list.data.iter().take(20).map(|t| {
                let side = if matches!(t.side, ClobSideType::Buy) { OrderSide::Buy } else { OrderSide::Sell };
                let mt = t.match_time;
                let session_ts = (mt.timestamp() / 900) * 900;
                let session_dt = chrono::DateTime::from_timestamp(session_ts, 0)
                    .unwrap_or(mt);
                RecentFill {
                    outcome: t.outcome.clone(),
                    side,
                    price:   t.price.to_string().parse().unwrap_or(0.0),
                    size:    t.size.to_string().parse().unwrap_or(0.0),
                    time:    mt.format("%H:%M:%S").to_string(),
                    session: session_dt.format("%H:%M").to_string(),
                }
            }).collect();

            info!("Fills recientes: {}", fills.len());
            let _ = tx.send(AppMsg::RecentFills(fills));
        }
        Ok(Err(e)) => warn!("trades: {}", e),
        Err(_)     => warn!("trades timeout"),
    }
}

/// Scalp automático: coloca BUY limit, espera fill, luego coloca SELL limit al precio objetivo.
/// Corre en un tokio::spawn independiente para no bloquear el loop principal.
#[allow(unused_assignments)]
async fn handle_scalp_buy(
    client:       &Client<Authenticated<Normal>>,
    signer:       &PrivateKeySigner,
    tx:           &mpsc::Sender<AppMsg>,
    broadcast_tx: &broadcast::Sender<String>,
    token_id:     U256,
    price:        f64,
    size:         f64,
    target_price: f64,
) {
    let send_msg = |msg: &str, bt: &broadcast::Sender<String>| {
        let _ = tx.send(AppMsg::OrderResult(msg.to_string()));
        if let Some(json) = AppMsg::OrderResult(msg.to_string()).to_json() {
            let _ = bt.send(json);
        }
    };

    let price_dec: Decimal = match format!("{:.2}", price).parse() {
        Ok(d) => d,
        Err(_) => { send_msg("⚡ Precio invalido", broadcast_tx); return; }
    };
    let size_dec: Decimal = match format!("{:.2}", size).parse() {
        Ok(d) => d,
        Err(_) => { send_msg("⚡ Tamaño invalido", broadcast_tx); return; }
    };

    let buy_result: Result<_> = async {
        let order = client.limit_order()
            .token_id(token_id)
            .order_type(OrderType::GTC)
            .price(price_dec)
            .size(size_dec)
            .side(ClobSide::Buy)
            .build().await?;
        let signed = client.sign(signer, order).await?;
        client.post_order(signed).await.map_err(|e| anyhow!(e))
    }.await;

    let order_id = match buy_result {
        Ok(r) if r.success => {
            let msg = format!(
                "⚡ SCALP BUY @ {:.4} enviado — esperando fill para vender @ {:.4}...",
                price, target_price
            );
            send_msg(&msg, broadcast_tx);
            r.order_id
        }
        Ok(r) => {
            send_msg(&format!("✗ SCALP rechazado: {}", r.error_msg.unwrap_or_default()), broadcast_tx);
            return;
        }
        Err(e) => {
            send_msg(&format!("✗ SCALP error: {e}"), broadcast_tx);
            return;
        }
    };

    let deadline = tokio::time::Instant::now() + Duration::from_secs(300);
    let filled_size;

    loop {
        if tokio::time::Instant::now() >= deadline {
            send_msg("⚠ SCALP timeout — fill no recibido en 5min", broadcast_tx);
            return;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;

        let req = OrdersRequest::builder().build();
        let result = tokio::time::timeout(Duration::from_secs(5), client.orders(&req, None)).await;
        match result {
            Ok(Ok(list)) => {
                let found = list.data.iter().find(|o| o.id == order_id);
                match found {
                    None => {
                        filled_size = size;
                        break;
                    }
                    Some(o) => {
                        let matched: f64 = o.size_matched.to_string().parse().unwrap_or(0.0);
                        let orig:    f64 = o.original_size.to_string().parse().unwrap_or(size);
                        if matched >= orig * 0.995 {
                            filled_size = matched;
                            break;
                        }
                    }
                }
            }
            _ => {}
        }
    }

    let sell_size = if filled_size > 0.0 { filled_size } else { size };
    let msg = format!(
        "✓ BUY filled! Colocando SELL @ {:.4} x {:.2} shares...",
        target_price, sell_size
    );
    send_msg(&msg, broadcast_tx);
    handle_limit_order(client, signer, tx, broadcast_tx, token_id, OrderSide::Sell, target_price, sell_size).await;
}

/// Obtiene el precio BTC/USD de Pyth Network al inicio del intervalo de 15min.
/// Polymarket usa Pyth como oráculo de precio para determinar el "Price to Beat".
/// Feed ID BTC/USD en Polygon: e62df6c8b4a85fe1a67db44dc12de5db330f7ac66b72dc658afedf0f4a415b43
async fn fetch_btc_open_price(market_end: &chrono::DateTime<Utc>) -> Option<f64> {
    let market_start = *market_end - chrono::Duration::minutes(15);
    let ts           = market_start.timestamp();  // Unix segundos

    const BTC_USD_FEED: &str =
        "e62df6c8b4a85fe1a67db44dc12de5db330f7ac66b72dc658afedf0f4a415b43";

    let url = format!(
        "https://hermes.pyth.network/v2/updates/price/{ts}?ids[]={BTC_USD_FEED}"
    );

    let resp = HttpClient::new()
        .get(&url)
        .send().await
        .ok()?;

    // Respuesta: { "parsed": [{ "price": { "price": "7227150000", "expo": -8, ... } }] }
    let json: serde_json::Value = resp.json().await.ok()?;

    let price_obj = json.get("parsed")?.get(0)?.get("price")?;

    let price_raw: f64 = price_obj.get("price")?.as_str()?.parse().ok()?;
    let expo:      i32 = price_obj.get("expo")?.as_i64()? as i32;

    let price = price_raw * 10_f64.powi(expo);
    if price > 0.0 { Some(price) } else { None }
}

// ─── Descubrimiento de mercado ────────────────────────────────────────────────

async fn discover_btc_market(gamma: &gamma::Client) -> Result<MarketInfo> {
    // Nivel 1: override manual
    if let Ok(raw) = std::env::var("BTC_TOKEN_ID") {
        let id_str  = raw.trim();
        let token_up: U256 = id_str.parse()
            .with_context(|| format!("BTC_TOKEN_ID inválido: '{id_str}'"))?;
        info!("Usando BTC_TOKEN_ID manual: {}", token_up);
        return Ok(MarketInfo {
            title:         "BTC 15-min (token manual)".into(),
            token_id_up:   token_up.to_string(),
            token_id_down: None,
            outcome_up:    "Up".into(),
            outcome_down:  "Down".into(),
            end_date:      next_15min_boundary(),
            active:        true,
            price_to_beat: None,
        });
    }

    // Nivel 2: slug determinista
    let now_ts  = Utc::now().timestamp();
    let current = (now_ts / 900) * 900;
    for start_ts in [current, current + 900, current - 900] {
        let slug = format!("btc-updown-15m-{start_ts}");
        info!("Probando slug: {slug}");
        match fetch_from_event(gamma, &slug).await {
            Ok(info) => { info!("Encontrado via slug: {slug}"); return Ok(info); }
            Err(e)   => warn!("Slug {slug}: {:#}", e),
        }
    }

    // Nivel 3: fallback
    warn!("Slug determinista falló — fallback en /markets");
    fetch_from_markets(gamma).await
}

async fn fetch_from_event(gamma: &gamma::Client, slug: &str) -> Result<MarketInfo> {
    let event = gamma
        .event_by_slug(&EventBySlugRequest::builder().slug(slug).build())
        .await
        .with_context(|| format!("event_by_slug({slug})"))?;

    if event.closed.unwrap_or(false) {
        return Err(anyhow!("Evento {slug} cerrado"));
    }

    let end_date = event.end_date.ok_or_else(|| anyhow!("Sin end_date"))?;
    let title    = event.title.clone().unwrap_or_else(|| slug.to_string());

    let markets = event.markets.as_ref()
        .ok_or_else(|| anyhow!("Sin mercados"))?;

    for market in markets {
        if let Some(ids) = &market.clob_token_ids {
            if ids.is_empty() { continue; }

            let token_id_up   = ids[0].to_string();
            let token_id_down = ids.get(1).map(|t| t.to_string());

            let outcomes = market.outcomes.as_ref();
            let outcome_up   = outcomes.and_then(|o| o.first()).cloned()
                .unwrap_or_else(|| "Up".to_string());
            let outcome_down = outcomes.and_then(|o| o.get(1)).cloned()
                .unwrap_or_else(|| "Down".to_string());

            // groupItemThreshold = precio BTC al inicio del intervalo (Price to Beat)
            let price_to_beat = market.group_item_threshold.as_deref()
                .and_then(|s| s.parse::<f64>().ok())
                .filter(|&p| p > 0.0);
            if let Some(p) = price_to_beat {
                info!("groupItemThreshold encontrado: {}", p);
            }

            return Ok(MarketInfo {
                title,
                token_id_up,
                token_id_down,
                outcome_up,
                outcome_down,
                end_date,
                active: !event.closed.unwrap_or(false),
                price_to_beat,
            });
        }
    }
    Err(anyhow!("Evento {slug} sin tokens"))
}

async fn fetch_from_markets(gamma: &gamma::Client) -> Result<MarketInfo> {
    let request = MarketsRequest::builder()
        .closed(false)
        .order("endDate".to_string())
        .ascending(true)
        .limit(200)
        .build();

    let markets = gamma.markets(&request).await
        .context("Error obteniendo mercados")?;

    info!("Fallback: {} mercados", markets.len());

    let now = Utc::now();
    let market = markets.iter()
        .filter(|m| m.active.unwrap_or(true) && !m.closed.unwrap_or(false))
        .filter(|m| {
            let q = m.question.as_deref().unwrap_or("").to_lowercase();
            q.contains("bitcoin") || q.contains("btc")
        })
        .filter(|m| m.end_date.map_or(false, |dt| dt.minute() % 15 == 0 && dt > now))
        .min_by_key(|m| m.end_date.map(|dt| (dt - now).num_seconds()).unwrap_or(i64::MAX))
        .ok_or_else(|| anyhow!(
            "No se encontró mercado BTC 15-min activo.\nAñade BTC_TOKEN_ID=<id> a tu .env"
        ))?;

    let ids = market.clob_token_ids.as_ref()
        .filter(|v| !v.is_empty())
        .ok_or_else(|| anyhow!("Mercado sin clob_token_ids"))?;

    let token_id_up   = ids[0].to_string();
    let token_id_down = ids.get(1).map(|t| t.to_string());

    let outcomes     = market.outcomes.as_ref();
    let outcome_up   = outcomes.and_then(|o| o.first()).cloned().unwrap_or_else(|| "Up".into());
    let outcome_down = outcomes.and_then(|o| o.get(1)).cloned().unwrap_or_else(|| "Down".into());

    let price_to_beat = market.group_item_threshold.as_deref()
        .and_then(|s| s.parse::<f64>().ok())
        .filter(|&p| p > 0.0);

    Ok(MarketInfo {
        title:         market.question.clone().unwrap_or_else(|| "Sin título".into()),
        token_id_up,
        token_id_down,
        outcome_up,
        outcome_down,
        end_date:      market.end_date.ok_or_else(|| anyhow!("Sin end_date"))?,
        active:        market.active.unwrap_or(true),
        price_to_beat,
    })
}

// ─── WebSocket: parsing ───────────────────────────────────────────────────────

fn handle_ws_text(text: &str, tx: &mpsc::Sender<AppMsg>, up_str: &str, down_str: Option<&str>, broadcast_tx: &broadcast::Sender<String>) {
    if let Ok(msgs) = serde_json::from_str::<Vec<serde_json::Value>>(text) {
        for msg in msgs { dispatch_ws_msg(&msg, tx, up_str, down_str, broadcast_tx); }
    } else if let Ok(msg) = serde_json::from_str::<serde_json::Value>(text) {
        dispatch_ws_msg(&msg, tx, up_str, down_str, broadcast_tx);
    }
}

fn dispatch_ws_msg(
    msg:      &serde_json::Value,
    tx:       &mpsc::Sender<AppMsg>,
    up_str:   &str,
    down_str: Option<&str>,
    broadcast_tx: &broadcast::Sender<String>,
) {
    let asset_id = msg.get("asset_id").and_then(|v| v.as_str()).unwrap_or("");
    let is_up    = asset_id == up_str;
    let is_down  = down_str.map_or(false, |d| asset_id == d);

    match msg.get("event_type").and_then(|v| v.as_str()) {
        Some("book") => {
            if let Some(snap) = parse_ws_book(msg) {
                if is_up        { let _ = tx.send(AppMsg::BookUp(snap.clone())); }
                else if is_down { let _ = tx.send(AppMsg::BookDown(snap.clone())); }

                if is_up {
                    if let Some(json) = AppMsg::BookUp(snap).to_json() {
                        let _ = broadcast_tx.send(json);
                    }
                } else if is_down {
                    if let Some(json) = AppMsg::BookDown(snap).to_json() {
                        let _ = broadcast_tx.send(json);
                    }
                }
            }
        }
        Some("last_trade_price") => {
            if let Some(p) = msg.get("price").and_then(|v| v.as_str())
                .and_then(|s| s.parse::<f64>().ok()).filter(|&p| p > 0.0)
            {
                if is_up        { let _ = tx.send(AppMsg::LastTradeUp(p)); }
                else if is_down { let _ = tx.send(AppMsg::LastTradeDown(p)); }

                if is_up {
                    if let Some(json) = AppMsg::LastTradeUp(p).to_json() {
                        let _ = broadcast_tx.send(json);
                    }
                } else if is_down {
                    if let Some(json) = AppMsg::LastTradeDown(p).to_json() {
                        let _ = broadcast_tx.send(json);
                    }
                }
            }
        }
        _ => {}
    }
}

fn parse_ws_book(msg: &serde_json::Value) -> Option<BookSnapshot> {
    let levels = |key: &str| -> Vec<PriceLevel> {
        msg.get(key)
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter().filter_map(|l| {
                    let price = l.get("price")?.as_str()?.parse().ok()?;
                    let size  = l.get("size")?.as_str()?.parse().ok()?;
                    Some(PriceLevel { price, size })
                }).collect()
            })
            .unwrap_or_default()
    };
    let bids = levels("bids");
    let asks = levels("asks");
    if bids.is_empty() && asks.is_empty() { return None; }
    Some(BookSnapshot { bids, asks })
}

// ─── WebSocket Server (para clientes Java) ─────────────────────────────────────

async fn run_websocket_server(
    addr: &str,
    broadcast_tx: broadcast::Sender<String>,
) -> Result<()> {
    let listener = TcpListener::bind(addr).await
        .map_err(|e| anyhow!("Failed to bind WebSocket server on {}: {}", addr, e))?;
    info!("WebSocket server listening on ws://{}", addr);

    loop {
        match listener.accept().await {
            Ok((stream, addr)) => {
                let broadcast_tx = broadcast_tx.clone();
                tokio::spawn(handle_ws_client(stream, addr, broadcast_tx));
            }
            Err(e) => {
                error!("WebSocket accept error: {}", e);
            }
        }
    }
}

async fn handle_ws_client(
    stream: TcpStream,
    addr: SocketAddr,
    broadcast_tx: broadcast::Sender<String>,
) {
    let ws_stream = match accept_async(stream).await {
        Ok(ws) => ws,
        Err(e) => {
            error!("WebSocket handshake failed for {}: {}", addr, e);
            return;
        }
    };

    info!("Java client connected: {}", addr);

    let (mut write, mut read) = ws_stream.split();
    let mut broadcast_rx = broadcast_tx.subscribe();

    let reader_handle = tokio::spawn(async move {
        while let Some(msg) = read.next().await {
            match msg {
                Ok(m) if m.is_text() => {
                    if let Ok(text) = m.into_text() {
                        info!("Received from Java client {}: {}", addr, text);
                    }
                }
                Ok(m) if m.is_close() => break,
                _ => {}
            }
        }
    });

    let writer_handle = tokio::spawn(async move {
        while let Ok(msg) = broadcast_rx.recv().await {
            if write.send(Message::Text(msg.into())).await.is_err() {
                break;
            }
        }
    });

    let _ = reader_handle.await;
    let _ = writer_handle.await;

    info!("Java client disconnected: {}", addr);
}

// ─── Broadcast AppMsg to JSON ─────────────────────────────────────────────────

impl AppMsg {
    pub fn to_json(&self) -> Option<String> {
        match self {
            AppMsg::Status(status) => {
                let msg = match status {
                    ConnStatus::Initializing => "Initializing",
                    ConnStatus::Authenticating => "Authenticating",
                    ConnStatus::FetchingMarkets => "FetchingMarkets",
                    ConnStatus::MarketFound(info) => &info.title,
                    ConnStatus::ConnectingWs => "ConnectingWs",
                    ConnStatus::Live => "LIVE",
                    ConnStatus::Reconnecting(n) => return Some(format!(r#"{{"type":"status","status":"Reconnecting","message":"Attempt {}"}}"#, n)),
                    ConnStatus::Error(e) => return Some(format!(r#"{{"type":"status","status":"Error","message":"{}"}}"#, e)),
                };
                Some(format!(r#"{{"type":"status","status":"{}"}}"#, msg))
            }
            AppMsg::BookUp(book) => {
                Some(format!(
                    r#"{{"type":"book","side":"up","book":{}}}"#,
                    book_to_json(book)
                ))
            }
            AppMsg::BookDown(book) => {
                Some(format!(
                    r#"{{"type":"book","side":"down","book":{}}}"#,
                    book_to_json(book)
                ))
            }
            AppMsg::LastTradeUp(price) => {
                Some(format!(r#"{{"type":"trade","side":"up","price":{}}}"#, price))
            }
            AppMsg::LastTradeDown(price) => {
                Some(format!(r#"{{"type":"trade","side":"down","price":{}}}"#, price))
            }
            AppMsg::Balance(bal) => {
                Some(format!(r#"{{"type":"balance","balance":{}}}"#, bal))
            }
            AppMsg::BtcOpen(price) => {
                Some(format!(r#"{{"type":"btc_price","open":{}}}"#, price))
            }
            AppMsg::BtcPrice(price) => {
                Some(format!(r#"{{"type":"btc_price","price":{}}}"#, price))
            }
            AppMsg::OrderResult(msg) => {
                Some(format!(r#"{{"type":"order_result","success":true,"message":"{}"}}"#, msg.replace('"', "\\\"")))
            }
            AppMsg::OpenOrders(orders) => {
                let orders_json: Vec<String> = orders.iter().map(|o| {
                    format!(r#"{{"id":"{}","outcome":"{}","side":"{}","price":{},"size_orig":{},"size_matched":{}}}"#,
                        o.id, o.outcome, match o.side { OrderSide::Buy => "BUY", OrderSide::Sell => "SELL" },
                        o.price, o.size_orig, o.size_matched)
                }).collect();
                Some(format!(r#"{{"type":"open_orders","orders":[{}]}}"#, orders_json.join(",")))
            }
            AppMsg::RecentFills(fills) => {
                let fills_json: Vec<String> = fills.iter().map(|f| {
                    format!(r#"{{"outcome":"{}","side":"{}","price":{},"size":{},"time":"{}","session":"{}"}}"#,
                        f.outcome, match f.side { OrderSide::Buy => "BUY", OrderSide::Sell => "SELL" },
                        f.price, f.size, f.time, f.session)
                }).collect();
                Some(format!(r#"{{"type":"recent_fills","fills":[{}]}}"#, fills_json.join(",")))
            }
            AppMsg::Candles(candles) => {
                let candles_json: Vec<String> = candles.iter().map(|c| {
                    format!(r#"{{"open_time":{},"open":{},"high":{},"low":{},"close":{},"volume":{}}}"#,
                        c.open_time, c.open, c.high, c.low, c.close, c.volume)
                }).collect();
                Some(format!(r#"{{"type":"candles","candles":[{}]}}"#, candles_json.join(",")))
            }
            AppMsg::CandleUpdate(candle) => {
                Some(format!(
                    r#"{{"type":"candle_update","candle":{{"open_time":{},"open":{},"high":{},"low":{},"close":{},"volume":{}}}}}"#,
                    candle.open_time, candle.open, candle.high, candle.low, candle.close, candle.volume
                ))
            }
        }
    }
}

fn book_to_json(book: &BookSnapshot) -> String {
    let bids: Vec<String> = book.bids.iter().map(|l| {
        format!(r#"{{"price":{},"size":{}}}"#, l.price, l.size)
    }).collect();
    let asks: Vec<String> = book.asks.iter().map(|l| {
        format!(r#"{{"price":{},"size":{}}}"#, l.price, l.size)
    }).collect();
    format!(r#"{{"bids":[{}],"asks":[{}]}}"#, bids.join(","), asks.join(","))
}

// ─── Utils ────────────────────────────────────────────────────────────────────

fn next_15min_boundary() -> DateTime<Utc> {
    let now_ts  = Utc::now().timestamp();
    let next_ts = ((now_ts / 900) + 1) * 900;
    DateTime::from_timestamp(next_ts, 0).unwrap_or_else(Utc::now)
}

fn convert_book(
    resp: &polymarket_client_sdk::clob::types::response::OrderBookSummaryResponse,
) -> BookSnapshot {
    let to_levels = |levels: &[polymarket_client_sdk::clob::types::response::OrderSummary]| {
        levels.iter().map(|l| PriceLevel {
            price: l.price.to_string().parse().unwrap_or(0.0),
            size:  l.size.to_string().parse().unwrap_or(0.0),
        }).collect()
    };
    BookSnapshot { bids: to_levels(&resp.bids), asks: to_levels(&resp.asks) }
}
