-- Candles OHLCV (time series BTC/USDT por intervalo)
CREATE TABLE IF NOT EXISTS candles (
    interval  VARCHAR(4)       NOT NULL,   -- '1s','1m','5m','15m','1h'
    open_time TIMESTAMPTZ      NOT NULL,
    open      DOUBLE PRECISION NOT NULL,
    high      DOUBLE PRECISION NOT NULL,
    low       DOUBLE PRECISION NOT NULL,
    close     DOUBLE PRECISION NOT NULL,
    volume    DOUBLE PRECISION NOT NULL,
    PRIMARY KEY (interval, open_time)
);
CREATE INDEX IF NOT EXISTS idx_candles_interval_time ON candles (interval, open_time DESC);

-- Ticks de precio BTC (muestreo cada N segundos para análisis)
CREATE TABLE IF NOT EXISTS btc_ticks (
    ts    TIMESTAMPTZ      NOT NULL DEFAULT NOW(),
    price DOUBLE PRECISION NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_btc_ticks_ts ON btc_ticks (ts DESC);

-- Historial de fills (trades ejecutados)
CREATE TABLE IF NOT EXISTS fills (
    id      BIGSERIAL        PRIMARY KEY,
    ts      TIMESTAMPTZ      NOT NULL DEFAULT NOW(),
    outcome VARCHAR(10)      NOT NULL,  -- 'Up' | 'Down'
    side    VARCHAR(4)       NOT NULL,  -- 'BUY' | 'SELL'
    price   DOUBLE PRECISION NOT NULL,
    size    DOUBLE PRECISION NOT NULL,
    session VARCHAR(5)       NOT NULL   -- '14:30'
);
CREATE INDEX IF NOT EXISTS idx_fills_ts ON fills (ts DESC);
