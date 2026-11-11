ALTER TABLE secrets_state ADD COLUMN key_id TEXT;
-- The original format guard remains in place; reject stale writers after rotation too.
CREATE FUNCTION enforce_current_secret_key() RETURNS TRIGGER LANGUAGE plpgsql AS $$
DECLARE expected TEXT; value JSONB;
BEGIN
 SELECT 'v1:' || key_id || ':' INTO expected FROM secrets_state WHERE singleton;
 IF expected IS NULL THEN RETURN NEW; END IF;
 IF TG_TABLE_NAME='channels' THEN
  FOR value IN SELECT item.value FROM jsonb_each(NEW.config) item WHERE item.key IN ('bot_token','token','access_token','password','webhook_secret','url','user','proxy_username','proxy_password','proxy_secret','api_hash','chat_access_hash','body_template') LOOP
   IF value<>'null'::jsonb AND value<>'""'::jsonb AND (jsonb_typeof(value)<>'string' OR left(value #>> '{}',length(expected))<>expected) THEN
    RAISE EXCEPTION 'Credential key identifier is stale; restart with the current SECRETS_KEY' USING ERRCODE='23514';
   END IF;
  END LOOP;
 ELSE
  IF (NEW.secret IS NOT NULL AND left(NEW.secret,length(expected))<>expected) OR (NEW.pending_secret IS NOT NULL AND left(NEW.pending_secret,length(expected))<>expected) THEN
   RAISE EXCEPTION 'Credential key identifier is stale; restart with the current SECRETS_KEY' USING ERRCODE='23514';
  END IF;
 END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER channels_current_key BEFORE INSERT OR UPDATE ON channels FOR EACH ROW EXECUTE FUNCTION enforce_current_secret_key();
CREATE TRIGGER totp_current_key BEFORE INSERT OR UPDATE ON user_totp FOR EACH ROW EXECUTE FUNCTION enforce_current_secret_key();
