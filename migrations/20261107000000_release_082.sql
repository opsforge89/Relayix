ALTER TABLE routes ADD COLUMN payload_rules JSONB NOT NULL DEFAULT '[]'::jsonb;
ALTER TABLE message_templates ADD COLUMN computed_status BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE message_templates ADD COLUMN passed_field TEXT NOT NULL DEFAULT 'passed';
ALTER TABLE message_templates ADD COLUMN failed_field TEXT NOT NULL DEFAULT 'failed';
CREATE TABLE message_template_versions (
 template_id UUID NOT NULL REFERENCES message_templates(id) ON DELETE CASCADE,
 version INTEGER NOT NULL,
 snapshot JSONB NOT NULL,
 created_by TEXT NOT NULL,
 created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
 PRIMARY KEY(template_id,version)
);
INSERT INTO message_template_versions(template_id,version,snapshot,created_by,created_at)
 SELECT id,version,to_jsonb(t),'Before version history',updated_at FROM message_templates t;
ALTER TABLE deliveries ADD COLUMN retry_generation INTEGER NOT NULL DEFAULT 0;
CREATE TABLE delivery_retry_batches (
 id UUID PRIMARY KEY,
 owner_key TEXT NOT NULL,
 delivery_ids UUID[] NOT NULL,
 queued INTEGER,
 expires_at TIMESTAMPTZ NOT NULL DEFAULT NOW()+INTERVAL '15 minutes',
 created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX delivery_retry_batches_expiry ON delivery_retry_batches(expires_at);
ALTER TABLE alert_policies ADD COLUMN owner_user_id UUID REFERENCES users(id) ON DELETE CASCADE;
ALTER TABLE alert_policies ADD COLUMN final_only BOOLEAN NOT NULL DEFAULT FALSE;
CREATE UNIQUE INDEX personal_alert_owner ON alert_policies(owner_user_id) WHERE owner_user_id IS NOT NULL;
ALTER TABLE alert_deliveries ADD COLUMN owner_user_id UUID REFERENCES users(id) ON DELETE CASCADE;
ALTER TABLE alert_incidents ADD COLUMN retry_generation INTEGER NOT NULL DEFAULT 0;
ALTER TABLE alert_incidents DROP CONSTRAINT alert_incidents_pkey;
ALTER TABLE alert_incidents ADD PRIMARY KEY(policy_id,delivery_id,retry_generation);
