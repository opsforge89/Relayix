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
DECLARE value JSONB;
BEGIN
 IF TG_TABLE_NAME='channels' THEN
  IF NOT NEW.secrets_encrypted AND EXISTS(SELECT 1 FROM secrets_state) THEN
   RAISE EXCEPTION 'Credentials must be encrypted; restart all application replicas with SECRETS_KEY' USING ERRCODE='23514';
  END IF;
  IF NEW.secrets_encrypted THEN
   FOR value IN SELECT item.value FROM jsonb_each(NEW.config) item WHERE item.key IN ('bot_token','token','access_token','password','webhook_secret','url','user','proxy_username','proxy_password','proxy_secret','api_hash','chat_access_hash','body_template') LOOP
    IF value<>'null'::jsonb AND value<>'""'::jsonb AND (jsonb_typeof(value)<>'string' OR (value #>> '{}') NOT LIKE 'v1:%') THEN
     RAISE EXCEPTION 'Encrypted credential format required' USING ERRCODE='23514';
    END IF;
   END LOOP;
  END IF;
 ELSE
  IF NOT NEW.secrets_encrypted AND EXISTS(SELECT 1 FROM secrets_state) AND (NEW.secret IS NOT NULL OR NEW.pending_secret IS NOT NULL) THEN
   RAISE EXCEPTION 'Credentials must be encrypted; restart all application replicas with SECRETS_KEY' USING ERRCODE='23514';
  END IF;
  IF NEW.secrets_encrypted AND ((NEW.secret IS NOT NULL AND NEW.secret NOT LIKE 'v1:%') OR (NEW.pending_secret IS NOT NULL AND NEW.pending_secret NOT LIKE 'v1:%')) THEN
   RAISE EXCEPTION 'Encrypted credential format required' USING ERRCODE='23514';
  END IF;
 END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER channels_secret_mode BEFORE INSERT OR UPDATE ON channels FOR EACH ROW EXECUTE FUNCTION enforce_secret_storage_mode();
CREATE TRIGGER totp_secret_mode BEFORE INSERT OR UPDATE ON user_totp FOR EACH ROW EXECUTE FUNCTION enforce_secret_storage_mode();
