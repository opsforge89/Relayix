ALTER TABLE routes ADD COLUMN rate_limit_per_minute integer NOT NULL DEFAULT 0 CHECK(rate_limit_per_minute BETWEEN 0 AND 1000000);
CREATE TABLE route_rate_windows (
 route_id uuid PRIMARY KEY REFERENCES routes(id) ON DELETE CASCADE,
 window_started_at timestamptz NOT NULL DEFAULT NOW(),
 requests integer NOT NULL DEFAULT 1 CHECK(requests>0)
);
