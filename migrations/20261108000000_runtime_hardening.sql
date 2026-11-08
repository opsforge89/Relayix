-- pg_trgm is a trusted PostgreSQL extension; the migration user needs CREATE on its database.
CREATE EXTENSION IF NOT EXISTS pg_trgm;
CREATE INDEX events_title_trgm ON events USING gin (title gin_trgm_ops);
CREATE INDEX events_message_trgm ON events USING gin (message gin_trgm_ops);
CREATE INDEX channels_name_trgm ON channels USING gin (name gin_trgm_ops);
CREATE INDEX routes_name_trgm ON routes USING gin (name gin_trgm_ops);
CREATE INDEX deliveries_error_trgm ON deliveries USING gin ((COALESCE(error,'')) gin_trgm_ops);
CREATE INDEX events_retention ON events(created_at);
-- One indexed row per distinct tag, maintained with event inserts/deletes.
CREATE TABLE event_tags (tag TEXT PRIMARY KEY, uses BIGINT NOT NULL CHECK (uses>0));
INSERT INTO event_tags SELECT tag,COUNT(*) FROM events CROSS JOIN LATERAL (SELECT DISTINCT jsonb_array_elements_text(tags) AS tag) t GROUP BY tag;
CREATE FUNCTION maintain_event_tags() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE item TEXT;
BEGIN
 IF TG_OP='INSERT' THEN
  FOR item IN SELECT DISTINCT jsonb_array_elements_text(NEW.tags) ORDER BY 1 LOOP
   INSERT INTO event_tags VALUES(item,1) ON CONFLICT(tag) DO UPDATE SET uses=event_tags.uses+1;
  END LOOP;
 ELSE
  FOR item IN SELECT DISTINCT jsonb_array_elements_text(OLD.tags) ORDER BY 1 LOOP
   -- Serialize deletion with concurrent inserts for the same tag.
   PERFORM 1 FROM event_tags WHERE tag=item FOR UPDATE;
   DELETE FROM event_tags WHERE tag=item AND uses=1;
   UPDATE event_tags SET uses=uses-1 WHERE tag=item;
  END LOOP;
 END IF;
 RETURN NULL;
END $$;
CREATE TRIGGER event_tags_insert AFTER INSERT ON events FOR EACH ROW EXECUTE FUNCTION maintain_event_tags();
CREATE TRIGGER event_tags_delete AFTER DELETE ON events FOR EACH ROW EXECUTE FUNCTION maintain_event_tags();
