# zzignal

Async trading backend for [Polymarket](https://polymarket.com) — REST API + WebSocket server + BTC market worker, built with Rust.

```
REST API  →  http://0.0.0.0:8080/api/...
WebSocket →  ws://0.0.0.0:8080/ws
```

## Features

- **Live order book** — real-time BTC UP/DOWN via Polymarket CLOB WebSocket
- **BTC price stream** — Binance aggTrade WebSocket with exponential backoff reconnect
- **OHLCV candles** — Binance kline stream, interval switchable at runtime (1s / 1m / 5m / 15m / 1h)
- **Order placement** — limit, market, and scalp mode (auto-sell on fill)
- **PostgreSQL persistence** — candles, BTC ticks, fills with automatic migrations
- **WebSocket broadcast** — snapshot on connect + live updates to all clients
- **REST API** — full CRUD for orders, positions, analysis, and P&L

## Stack

| Component | Crate |
|-----------|-------|
| Async runtime | `tokio` 1 |
| REST + WebSocket API | `axum` 0.8 |
| Polymarket | `polymarket-client-sdk` 0.4 |
| Ethereum signer | `alloy` 1.8 |
| Database | `sqlx` 0.8 (PostgreSQL) |
| HTTP client | `reqwest` 0.13 (rustls) |
| Serialization | `serde` + `serde_json` |

**MSRV: Rust 1.91**

## Quick start

```bash
cargo install zzignal
```

Or from source:

```bash
git clone https://github.com/compascafe/zzignal
cd zzignal
cp .env.example .env   # fill in your credentials
cargo run --release
```

## Configuration

Create a `.env` file:

```env
POLYMARKET_PRIVATE_KEY=0x...      # Ethereum private key (hex)
CLOB_API_KEY=...                  # Polymarket L2 API key
CLOB_API_SECRET=...               # Polymarket L2 API secret
CLOB_API_PASSPHRASE=...           # Polymarket L2 passphrase
DATABASE_URL=postgres://user:pass@localhost/zzignal  # optional
```

The wallet address is **derived** from the private key — never stored or logged in plain text.

## API

| Method | Route | Description |
|--------|-------|-------------|
| GET | `/api/status` | Worker connection status |
| GET | `/api/market` | Active BTC 15-min market info |
| GET | `/api/balance` | USDC balance |
| GET | `/api/btc` | Current BTC price |
| GET | `/api/book/up` | UP outcome order book |
| GET | `/api/book/down` | DOWN outcome order book |
| GET | `/api/candles` | Live candle buffer |
| POST | `/api/candles/interval` | Switch interval `{"interval":"1m"}` |
| GET | `/api/orders` | Open orders |
| POST | `/api/orders/limit` | Place limit order |
| POST | `/api/orders/market` | Place market order |
| POST | `/api/orders/scalp` | Scalp buy (auto-sell on fill) |
| DELETE | `/api/orders/{id}` | Cancel order |
| DELETE | `/api/orders` | Cancel all orders |
| GET | `/api/fills` | Recent fills |
| GET | `/api/analysis/candles` | Historical candles from DB |
| GET | `/api/analysis/pnl` | P&L from DB |
| GET | `/ws` | WebSocket endpoint |

## WebSocket

Connect to `/ws` to receive a full snapshot on connect, then live updates:

```json
{ "BtcPrice": 94250.5 }
{ "BookUp": { "bids": [...], "asks": [...] } }
{ "CandleUpdate": { "open_time": 1234567890, "open": 94100, "high": 94300, "low": 94050, "close": 94250, "volume": 12.3 } }
```

Send commands as JSON:

```json
{ "PlaceLimitOrder": { "side": "Buy", "outcome": "Up", "price": 0.62, "size": 10.0 } }
{ "CancelMarket": null }
```

## License

MIT — see [LICENSE](LICENSE)
