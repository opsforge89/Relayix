CREATE TABLE attachment_storage (
 singleton BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK(singleton),
 bytes_used BIGINT NOT NULL DEFAULT 0 CHECK(bytes_used>=0),
 limit_bytes BIGINT NOT NULL DEFAULT 1073741824 CHECK(limit_bytes>0)
);
INSERT INTO attachment_storage(singleton) VALUES(TRUE);
CREATE TABLE event_attachments (
 id UUID PRIMARY KEY,
 event_id UUID NOT NULL REFERENCES events(id) ON DELETE CASCADE,
 position INT NOT NULL CHECK(position BETWEEN 0 AND 4),
 name TEXT NOT NULL,
 content_type TEXT NOT NULL,
 sha256 TEXT NOT NULL,
 data BYTEA NOT NULL CHECK(octet_length(data)<=10485760),
 UNIQUE(event_id,position)
);
CREATE INDEX event_attachments_event ON event_attachments(event_id);
CREATE FUNCTION account_attachment_storage() RETURNS TRIGGER LANGUAGE plpgsql AS $$
DECLARE charge BIGINT;
BEGIN
 IF TG_OP='INSERT' THEN
  charge := GREATEST(octet_length(NEW.data),4096);
  UPDATE attachment_storage SET bytes_used=bytes_used+charge WHERE singleton AND bytes_used+charge<=limit_bytes;
  IF NOT FOUND THEN RAISE EXCEPTION 'Attachment storage budget exceeded' USING ERRCODE='P0001'; END IF;
  RETURN NEW;
 END IF;
 charge := GREATEST(octet_length(OLD.data),4096);
 UPDATE attachment_storage SET bytes_used=bytes_used-charge WHERE singleton;
 RETURN OLD;
END $$;
CREATE TRIGGER attachment_storage_insert BEFORE INSERT ON event_attachments FOR EACH ROW EXECUTE FUNCTION account_attachment_storage();
CREATE TRIGGER attachment_storage_delete AFTER DELETE ON event_attachments FOR EACH ROW EXECUTE FUNCTION account_attachment_storage();
CREATE TABLE attachment_delivery_steps (
 delivery_id UUID NOT NULL REFERENCES deliveries(id) ON DELETE CASCADE,
 fingerprint TEXT NOT NULL,
 step TEXT NOT NULL,
 remote_id TEXT NOT NULL,
 PRIMARY KEY(delivery_id,fingerprint,step)
);
