use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::{PgPool, Row};

use crate::worker::{Candle, OrderSide, RecentFill};

// ─── Migraciones ─────────────────────────────────────────────────────────────

pub async fn run_migrations(pool: &PgPool) -> Result<()> {
    sqlx::migrate!("./migrations").run(pool).await?;
    Ok(())
}

// ─── Candles ─────────────────────────────────────────────────────────────────

pub async fn upsert_candle(pool: Option<&PgPool>, interval: &str, c: &Candle) -> Result<()> {
    let Some(pool) = pool else { return Ok(()) };
    upsert_candle_inner(pool, interval, c).await
}

async fn upsert_candle_inner(pool: &PgPool, interval: &str, c: &Candle) -> Result<()> {
    let open_time = DateTime::<Utc>::from_timestamp_millis(c.open_time)
        .unwrap_or_default();
    sqlx::query(
        r#"
        INSERT INTO candles (interval, open_time, open, high, low, close, volume)
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        ON CONFLICT (interval, open_time) DO UPDATE SET
            open   = EXCLUDED.open,
            high   = EXCLUDED.high,
            low    = EXCLUDED.low,
            close  = EXCLUDED.close,
            volume = EXCLUDED.volume
        "#,
    )
    .bind(interval)
    .bind(open_time)
    .bind(c.open)
    .bind(c.high)
    .bind(c.low)
    .bind(c.close)
    .bind(c.volume)
    .execute(pool)
    .await?;
    Ok(())
}

// ─── BTC ticks ───────────────────────────────────────────────────────────────

pub async fn insert_btc_tick(pool: Option<&PgPool>, price: f64) -> Result<()> {
    let Some(pool) = pool else { return Ok(()) };
    sqlx::query("INSERT INTO btc_ticks (price) VALUES ($1)")
        .bind(price)
        .execute(pool)
        .await?;
    Ok(())
}

// ─── Fills ───────────────────────────────────────────────────────────────────

pub async fn insert_fill(pool: Option<&PgPool>, fill: &RecentFill) -> Result<()> {
    let Some(pool) = pool else { return Ok(()) };
    let side = match fill.side {
        OrderSide::Buy  => "BUY",
        OrderSide::Sell => "SELL",
    };
    sqlx::query(
        "INSERT INTO fills (outcome, side, price, size, session) VALUES ($1,$2,$3,$4,$5)",
    )
    .bind(&fill.outcome)
    .bind(side)
    .bind(fill.price)
    .bind(fill.size)
    .bind(&fill.session)
    .execute(pool)
    .await?;
    Ok(())
}

// ─── Queries de análisis ──────────────────────────────────────────────────────

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct CandleRow {
    pub interval:  String,
    pub open_time: DateTime<Utc>,
    pub open:      f64,
    pub high:      f64,
    pub low:       f64,
    pub close:     f64,
    pub volume:    f64,
}

pub async fn query_candles(
    pool:     Option<&PgPool>,
    interval: &str,
    limit:    i64,
    from:     Option<DateTime<Utc>>,
    to:       Option<DateTime<Utc>>,
) -> Result<Vec<CandleRow>> {
    let Some(pool) = pool else { return Ok(vec![]) };
    let rows = sqlx::query_as::<_, CandleRow>(
        r#"
        SELECT interval, open_time, open, high, low, close, volume
        FROM   candles
        WHERE  interval = $1
          AND  ($2::timestamptz IS NULL OR open_time >= $2)
          AND  ($3::timestamptz IS NULL OR open_time <= $3)
        ORDER  BY open_time DESC
        LIMIT  $4
        "#,
    )
    .bind(interval)
    .bind(from)
    .bind(to)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct FillRow {
    pub id:      i64,
    pub ts:      DateTime<Utc>,
    pub outcome: String,
    pub side:    String,
    pub price:   f64,
    pub size:    f64,
    pub session: String,
}

pub async fn query_fills(pool: Option<&PgPool>, limit: i64) -> Result<Vec<FillRow>> {
    let Some(pool) = pool else { return Ok(vec![]) };
    let rows = sqlx::query_as::<_, FillRow>(
        "SELECT id,ts,outcome,side,price,size,session FROM fills ORDER BY ts DESC LIMIT $1",
    )
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

#[derive(Debug, Serialize)]
pub struct PnlRow {
    pub outcome:      String,
    pub total_bought: f64,
    pub total_sold:   f64,
    pub net_position: f64,
    pub realized_pnl: f64,
}

pub async fn query_pnl(pool: Option<&PgPool>) -> Result<Vec<PnlRow>> {
    let Some(pool) = pool else { return Ok(vec![]) };
    let rows = sqlx::query(
        r#"
        SELECT
            outcome,
            COALESCE(SUM(CASE WHEN side='BUY'  THEN size  ELSE 0 END), 0) AS total_bought,
            COALESCE(SUM(CASE WHEN side='SELL' THEN size  ELSE 0 END), 0) AS total_sold,
            COALESCE(SUM(CASE WHEN side='BUY'  THEN size  ELSE -size END), 0) AS net_position,
            COALESCE(SUM(CASE WHEN side='SELL' THEN price*size ELSE -price*size END), 0) AS realized_pnl
        FROM fills
        GROUP BY outcome
        ORDER BY outcome
        "#,
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .iter()
        .map(|r| PnlRow {
            outcome:      r.get::<String, _>("outcome"),
            total_bought: r.get::<f64, _>("total_bought"),
            total_sold:   r.get::<f64, _>("total_sold"),
            net_position: r.get::<f64, _>("net_position"),
            realized_pnl: r.get::<f64, _>("realized_pnl"),
        })
        .collect())
}
