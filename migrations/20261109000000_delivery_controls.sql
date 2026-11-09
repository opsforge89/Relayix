ALTER TABLE channels ADD COLUMN manual_paused BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE channels ADD COLUMN manual_pause_until TIMESTAMPTZ;
ALTER TABLE routes ADD COLUMN delivery_settings JSONB NOT NULL DEFAULT '{}'::jsonb;
ALTER TABLE deliveries ADD COLUMN not_before TIMESTAMPTZ NOT NULL DEFAULT NOW();
ALTER TABLE alert_deliveries ADD COLUMN not_before TIMESTAMPTZ NOT NULL DEFAULT NOW();
CREATE TABLE event_groups (
 id UUID PRIMARY KEY,
 route_id UUID NOT NULL REFERENCES routes(id) ON DELETE CASCADE,
 leader_event_id UUID NOT NULL REFERENCES events(id) ON DELETE CASCADE,
 group_key TEXT NOT NULL,
 kind TEXT NOT NULL CHECK(kind IN ('dedup','digest')),
 closes_at TIMESTAMPTZ NOT NULL,
 occurrences INT NOT NULL DEFAULT 1,
 created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX event_groups_open ON event_groups(route_id,group_key,closes_at);
ALTER TABLE events ADD COLUMN group_id UUID REFERENCES event_groups(id) ON DELETE SET NULL;
ALTER TABLE events ADD COLUMN deliver_at TIMESTAMPTZ;
CREATE INDEX events_group ON events(group_id,created_at,id) WHERE group_id IS NOT NULL;
CREATE TABLE user_totp (
 user_id UUID PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
 generation UUID NOT NULL DEFAULT gen_random_uuid(),
 secret TEXT,
 pending_secret TEXT,
 pending_expires_at TIMESTAMPTZ,
 last_step BIGINT NOT NULL DEFAULT -1,
 failures INT NOT NULL DEFAULT 0,
 locked_until TIMESTAMPTZ,
 enabled_at TIMESTAMPTZ
);
CREATE TABLE totp_recovery_codes (
 user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
 code_hash TEXT NOT NULL,
 PRIMARY KEY(user_id,code_hash)
);
ALTER TABLE sessions ADD COLUMN mfa_generation UUID;
