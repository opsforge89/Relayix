-- Bound active-work counts used by the shared per-channel claim gate.
CREATE INDEX deliveries_channel_active ON deliveries(channel_id,locked_at) WHERE status='processing';
CREATE INDEX alerts_channel_active ON alert_deliveries(channel_id,locked_at) WHERE status='processing';
CREATE INDEX deliveries_channel_waiting ON deliveries(channel_id,created_at) WHERE status IN ('queued','processing');
CREATE INDEX alerts_channel_waiting ON alert_deliveries(channel_id,created_at) WHERE status IN ('queued','processing');
