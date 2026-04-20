use std::sync::Arc;

use axum::{
    Router,
    extract::{Path, Query, State, WebSocketUpgrade},
    response::Response,
    routing::{delete, get, post},
    Json,
};
use axum::extract::ws::{Message, WebSocket};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use tower_http::cors::CorsLayer;

use crate::db;
use crate::state::AppState;
use crate::worker::{CandleInterval, CmdMsg, OrderSide, Outcome};

// ─── Router ───────────────────────────────────────────────────────────────────

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        // Status / mercado
        .route("/api/status",          get(get_status))
        .route("/api/market",          get(get_market))
        .route("/api/balance",         get(get_balance))
        .route("/api/btc",             get(get_btc))
        // Order book
        .route("/api/book/up",         get(get_book_up))
        .route("/api/book/down",       get(get_book_down))
        // Candles (live desde estado en memoria)
        .route("/api/candles",         get(get_candles))
        .route("/api/candles/interval",post(set_interval))
        // Órdenes abiertas
        .route("/api/orders",          get(get_orders))
        .route("/api/orders",          delete(cancel_all_orders))
        .route("/api/orders/{id}",     delete(cancel_order))
        // Colocar órdenes
        .route("/api/orders/limit",    post(post_limit_order))
        .route("/api/orders/market",   post(post_market_order))
        .route("/api/orders/scalp",    post(post_scalp_order))
        // Fills
        .route("/api/fills",           get(get_fills))
        // Análisis histórico (PostgreSQL)
        .route("/api/analysis/candles",get(analysis_candles))
        .route("/api/analysis/pnl",    get(analysis_pnl))
        .route("/api/analysis/fills",  get(analysis_fills))
        // WebSocket
        .route("/ws",                  get(ws_handler))
        .layer(CorsLayer::permissive())
        .with_state(state)
}

// ─── Status & Mercado ─────────────────────────────────────────────────────────

async fn get_status(State(s): State<Arc<AppState>>) -> Json<Value> {
    Json(json!({ "status": *s.status.read().await }))
}

async fn get_market(State(s): State<Arc<AppState>>) -> Json<Value> {
    let m = s.market.read().await;
    match &*m {
        Some(m) => Json(json!({
            "title":         m.title,
            "outcome_up":    m.outcome_up,
            "outcome_down":  m.outcome_down,
            "price_to_beat": m.price_to_beat,
            "end_date":      m.end_date,
            "active":        m.active,
        })),
        None => Json(json!(null)),
    }
}

async fn get_balance(State(s): State<Arc<AppState>>) -> Json<Value> {
    Json(json!({ "balance": *s.balance.read().await }))
}

async fn get_btc(State(s): State<Arc<AppState>>) -> Json<Value> {
    Json(json!({
        "price": *s.btc_price.read().await,
        "open":  *s.btc_open.read().await,
    }))
}

// ─── Order Book ───────────────────────────────────────────────────────────────

async fn get_book_up(State(s): State<Arc<AppState>>) -> Json<Value> {
    let guard = s.book_up.read().await;
    Json(book_snapshot_to_json(&guard))
}

async fn get_book_down(State(s): State<Arc<AppState>>) -> Json<Value> {
    let guard = s.book_down.read().await;
    Json(book_snapshot_to_json(&guard))
}

fn book_snapshot_to_json(book: &Option<crate::worker::BookSnapshot>) -> Value {
    match book {
        Some(b) => json!({
            "bids": b.bids.iter().map(|l| json!({"price": l.price, "size": l.size})).collect::<Vec<_>>(),
            "asks": b.asks.iter().map(|l| json!({"price": l.price, "size": l.size})).collect::<Vec<_>>(),
        }),
        None => json!(null),
    }
}

// ─── Candles ──────────────────────────────────────────────────────────────────

async fn get_candles(State(s): State<Arc<AppState>>) -> Json<Value> {
    let candles = s.candles.read().await;
    let arr: Vec<Value> = candles.iter().map(|c| json!({
        "open_time": c.open_time,
        "open":  c.open,
        "high":  c.high,
        "low":   c.low,
        "close": c.close,
        "volume":c.volume,
    })).collect();
    Json(json!(arr))
}

#[derive(Deserialize)]
struct IntervalBody {
    interval: String,  // "1s" | "1m" | "5m" | "15m" | "1h"
}

async fn set_interval(
    State(s):    State<Arc<AppState>>,
    Json(body):  Json<IntervalBody>,
) -> Json<Value> {
    let iv = match body.interval.as_str() {
        "1s"  => CandleInterval::OneSecond,
        "1m"  => CandleInterval::OneMinute,
        "5m"  => CandleInterval::FiveMinutes,
        "15m" => CandleInterval::FifteenMinutes,
        "1h"  => CandleInterval::OneHour,
        other => return Json(json!({"ok": false, "error": format!("intervalo desconocido: {other}")})),
    };
    if let Ok(mut guard) = s.interval_arc.lock() {
        *guard = iv;
    }
    Json(json!({"ok": true, "interval": body.interval}))
}

// ─── Órdenes ──────────────────────────────────────────────────────────────────

async fn get_orders(State(s): State<Arc<AppState>>) -> Json<Value> {
    let orders = s.open_orders.read().await;
    let arr: Vec<Value> = orders.iter().map(|o| json!({
        "id":           o.id,
        "outcome":      o.outcome,
        "side":         side_str(o.side),
        "price":        o.price,
        "size_orig":    o.size_orig,
        "size_matched": o.size_matched,
    })).collect();
    Json(json!(arr))
}

#[derive(Deserialize)]
struct LimitOrderReq {
    side:    String,
    outcome: String,
    price:   f64,
    size:    f64,
}

async fn post_limit_order(
    State(s):   State<Arc<AppState>>,
    Json(body): Json<LimitOrderReq>,
) -> Json<Value> {
    let (side, outcome) = match parse_args(&body.side, &body.outcome) {
        Ok(v) => v,
        Err(e) => return Json(json!({"ok": false, "error": e})),
    };
    let _ = s.cmd_tx.send(CmdMsg::PlaceLimitOrder { side, outcome, price: body.price, size: body.size });
    Json(json!({"ok": true}))
}

#[derive(Deserialize)]
struct MarketOrderReq {
    side:        String,
    outcome:     String,
    amount_usdc: f64,
}

async fn post_market_order(
    State(s):   State<Arc<AppState>>,
    Json(body): Json<MarketOrderReq>,
) -> Json<Value> {
    let (side, outcome) = match parse_args(&body.side, &body.outcome) {
        Ok(v) => v,
        Err(e) => return Json(json!({"ok": false, "error": e})),
    };
    let _ = s.cmd_tx.send(CmdMsg::PlaceMarketOrder { side, outcome, amount_usdc: body.amount_usdc });
    Json(json!({"ok": true}))
}

#[derive(Deserialize)]
struct ScalpReq {
    outcome:      String,
    price:        f64,
    size:         f64,
    target_price: f64,
}

async fn post_scalp_order(
    State(s):   State<Arc<AppState>>,
    Json(body): Json<ScalpReq>,
) -> Json<Value> {
    let outcome = match parse_outcome(&body.outcome) {
        Ok(o) => o,
        Err(e) => return Json(json!({"ok": false, "error": e})),
    };
    let _ = s.cmd_tx.send(CmdMsg::ScalpBuy {
        outcome,
        price:        body.price,
        size:         body.size,
        target_price: body.target_price,
    });
    Json(json!({"ok": true}))
}

async fn cancel_order(
    State(s): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Json<Value> {
    let _ = s.cmd_tx.send(CmdMsg::CancelOrder { order_id: id });
    Json(json!({"ok": true}))
}

async fn cancel_all_orders(State(s): State<Arc<AppState>>) -> Json<Value> {
    let _ = s.cmd_tx.send(CmdMsg::CancelMarket);
    Json(json!({"ok": true}))
}

// ─── Fills ────────────────────────────────────────────────────────────────────

async fn get_fills(State(s): State<Arc<AppState>>) -> Json<Value> {
    let fills = s.recent_fills.read().await;
    let arr: Vec<Value> = fills.iter().map(|f| json!({
        "outcome": f.outcome,
        "side":    side_str(f.side),
        "price":   f.price,
        "size":    f.size,
        "time":    f.time,
        "session": f.session,
    })).collect();
    Json(json!(arr))
}

// ─── Análisis (PostgreSQL) ────────────────────────────────────────────────────

#[derive(Deserialize)]
struct CandleQuery {
    interval: Option<String>,
    limit:    Option<i64>,
    from:     Option<DateTime<Utc>>,
    to:       Option<DateTime<Utc>>,
}

async fn analysis_candles(
    State(s): State<Arc<AppState>>,
    Query(q): Query<CandleQuery>,
) -> Json<Value> {
    let interval = q.interval.as_deref().unwrap_or("1m");
    let limit    = q.limit.unwrap_or(500).min(5000);
    match db::query_candles(s.db.as_ref(), interval, limit, q.from, q.to).await {
        Ok(rows) => Json(json!(rows)),
        Err(e)   => Json(json!({"error": e.to_string()})),
    }
}

async fn analysis_pnl(State(s): State<Arc<AppState>>) -> Json<Value> {
    match db::query_pnl(s.db.as_ref()).await {
        Ok(rows) => Json(json!(rows)),
        Err(e)   => Json(json!({"error": e.to_string()})),
    }
}

#[derive(Deserialize)]
struct FillQuery {
    limit: Option<i64>,
}

async fn analysis_fills(
    State(s): State<Arc<AppState>>,
    Query(q): Query<FillQuery>,
) -> Json<Value> {
    let limit = q.limit.unwrap_or(100).min(1000);
    match db::query_fills(s.db.as_ref(), limit).await {
        Ok(rows) => Json(json!(rows)),
        Err(e)   => Json(json!({"error": e.to_string()})),
    }
}

// ─── WebSocket ────────────────────────────────────────────────────────────────

async fn ws_handler(
    ws:       WebSocketUpgrade,
    State(s): State<Arc<AppState>>,
) -> Response {
    ws.on_upgrade(move |socket| handle_ws_socket(socket, s))
}

async fn handle_ws_socket(mut socket: WebSocket, state: Arc<AppState>) {
    let mut rx = state.broadcast_tx.subscribe();

    // Snapshot inicial al conectar
    let snap = build_snapshot(&state).await;
    if socket.send(Message::Text(snap.into())).await.is_err() {
        return;
    }

    loop {
        tokio::select! {
            // Reenviar broadcasts al cliente
            result = rx.recv() => {
                match result {
                    Ok(msg) => {
                        if socket.send(Message::Text(msg.into())).await.is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
            // Recibir comandos del cliente
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Text(text))) => handle_ws_cmd(&text, &state),
                    Some(Ok(Message::Ping(d)))    => { let _ = socket.send(Message::Pong(d)).await; }
                    Some(Ok(Message::Close(_))) | None => break,
                    _ => {}
                }
            }
        }
    }
}

async fn build_snapshot(state: &AppState) -> String {
    let status  = state.status.read().await.clone();
    let btc     = *state.btc_price.read().await;
    let balance = *state.balance.read().await;
    json!({
        "type":    "snapshot",
        "status":  status,
        "btc":     btc,
        "balance": balance,
    }).to_string()
}

fn handle_ws_cmd(text: &str, state: &AppState) {
    let Ok(v) = serde_json::from_str::<Value>(text) else { return };
    let cmd_type = v.get("type").and_then(|t| t.as_str()).unwrap_or("");

    match cmd_type {
        "limit" => {
            let side    = v["side"].as_str().and_then(|s| parse_side(s).ok());
            let outcome = v["outcome"].as_str().and_then(|o| parse_outcome(o).ok());
            if let (Some(side), Some(outcome), Some(price), Some(size)) =
                (side, outcome, v["price"].as_f64(), v["size"].as_f64())
            {
                let _ = state.cmd_tx.send(CmdMsg::PlaceLimitOrder { side, outcome, price, size });
            }
        }
        "market" => {
            let side    = v["side"].as_str().and_then(|s| parse_side(s).ok());
            let outcome = v["outcome"].as_str().and_then(|o| parse_outcome(o).ok());
            if let (Some(side), Some(outcome), Some(amount_usdc)) =
                (side, outcome, v["amount_usdc"].as_f64())
            {
                let _ = state.cmd_tx.send(CmdMsg::PlaceMarketOrder { side, outcome, amount_usdc });
            }
        }
        "scalp" => {
            let outcome      = v["outcome"].as_str().and_then(|o| parse_outcome(o).ok());
            let price        = v["price"].as_f64();
            let size         = v["size"].as_f64();
            let target_price = v["target_price"].as_f64();
            if let (Some(outcome), Some(price), Some(size), Some(target_price)) =
                (outcome, price, size, target_price)
            {
                let _ = state.cmd_tx.send(CmdMsg::ScalpBuy { outcome, price, size, target_price });
            }
        }
        "cancel" => {
            if let Some(id) = v["order_id"].as_str() {
                let _ = state.cmd_tx.send(CmdMsg::CancelOrder { order_id: id.to_string() });
            }
        }
        "cancel_all" => {
            let _ = state.cmd_tx.send(CmdMsg::CancelMarket);
        }
        "set_interval" => {
            if let Some(iv_str) = v["interval"].as_str() {
                let iv = match iv_str {
                    "1s"  => Some(CandleInterval::OneSecond),
                    "1m"  => Some(CandleInterval::OneMinute),
                    "5m"  => Some(CandleInterval::FiveMinutes),
                    "15m" => Some(CandleInterval::FifteenMinutes),
                    "1h"  => Some(CandleInterval::OneHour),
                    _     => None,
                };
                if let (Some(iv), Ok(mut guard)) = (iv, state.interval_arc.lock()) {
                    *guard = iv;
                }
            }
        }
        _ => {}
    }
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

fn parse_side(s: &str) -> Result<OrderSide, String> {
    match s.to_lowercase().as_str() {
        "buy"  => Ok(OrderSide::Buy),
        "sell" => Ok(OrderSide::Sell),
        other  => Err(format!("side inválido: {other}")),
    }
}

fn parse_outcome(s: &str) -> Result<Outcome, String> {
    match s.to_lowercase().as_str() {
        "up"   => Ok(Outcome::Up),
        "down" => Ok(Outcome::Down),
        other  => Err(format!("outcome inválido: {other}")),
    }
}

fn parse_args(side: &str, outcome: &str) -> Result<(OrderSide, Outcome), String> {
    Ok((parse_side(side)?, parse_outcome(outcome)?))
}

fn side_str(s: OrderSide) -> &'static str {
    match s { OrderSide::Buy => "BUY", OrderSide::Sell => "SELL" }
}
