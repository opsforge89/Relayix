-- Shared account backpressure across client IPs and application replicas.
-- Existing hashed IP/username windows remain compatible.
ALTER TABLE login_attempts ADD COLUMN next_allowed_at timestamptz NOT NULL DEFAULT NOW();
