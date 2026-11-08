CREATE TABLE channel_health (
 channel_id UUID PRIMARY KEY REFERENCES channels(id) ON DELETE CASCADE,
 consecutive_failures INT NOT NULL DEFAULT 0,
 failure_signature TEXT,
 last_error TEXT,
 paused_until TIMESTAMPTZ,
 updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
ALTER TABLE deliveries ADD COLUMN error_category TEXT;
ALTER TABLE delivery_attempts ADD COLUMN error_category TEXT;
ALTER TABLE alert_deliveries ADD COLUMN error_category TEXT;
