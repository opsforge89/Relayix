ALTER TABLE events ADD COLUMN expires_at TIMESTAMPTZ;
ALTER TABLE deliveries ADD COLUMN expires_at TIMESTAMPTZ;
ALTER TABLE alert_deliveries ADD COLUMN expires_at TIMESTAMPTZ;
CREATE INDEX deliveries_expiry ON deliveries(expires_at,id) WHERE expires_at IS NOT NULL AND status IN ('queued','processing');
CREATE INDEX alerts_expiry ON alert_deliveries(expires_at,id) WHERE expires_at IS NOT NULL AND status IN ('queued','processing');
CREATE TABLE audit_log (
 id BIGSERIAL PRIMARY KEY,
 actor_id UUID,
 actor TEXT NOT NULL,
 auth_method TEXT NOT NULL,
 action TEXT NOT NULL,
 target_type TEXT NOT NULL,
 target_id UUID,
 affected BIGINT,
 created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX audit_log_time ON audit_log(created_at DESC,id DESC);
CREATE INDEX audit_log_target ON audit_log(target_id,created_at DESC);
ALTER TABLE channels ADD COLUMN secrets_encrypted BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE user_totp ADD COLUMN secrets_encrypted BOOLEAN NOT NULL DEFAULT FALSE;
CREATE TABLE secrets_state (
 singleton BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK(singleton),
 key_check TEXT NOT NULL
);

CREATE FUNCTION enforce_secret_storage_mode() RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
 IF NOT NEW.secrets_encrypted AND EXISTS(SELECT 1 FROM secrets_state) THEN
  IF TG_TABLE_NAME='channels' THEN
   RAISE EXCEPTION 'Credentials must be encrypted; restart all application replicas with SECRETS_KEY' USING ERRCODE='23514';
  ELSIF NEW.secret IS NOT NULL OR NEW.pending_secret IS NOT NULL THEN
   RAISE EXCEPTION 'Credentials must be encrypted; restart all application replicas with SECRETS_KEY' USING ERRCODE='23514';
  END IF;
 END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER channels_secret_mode BEFORE INSERT OR UPDATE ON channels FOR EACH ROW EXECUTE FUNCTION enforce_secret_storage_mode();
CREATE TRIGGER totp_secret_mode BEFORE INSERT OR UPDATE ON user_totp FOR EACH ROW EXECUTE FUNCTION enforce_secret_storage_mode();
