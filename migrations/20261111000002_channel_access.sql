-- NULL ownership is reserved for administrator-managed legacy/imported resources.
-- Deleted owners are retained as UUID tombstones: deletion must not grant system access.
ALTER TABLE channels ADD COLUMN owner_user_id UUID;
ALTER TABLE routes ADD COLUMN owner_user_id UUID;
CREATE INDEX channels_owner_idx ON channels(owner_user_id);
CREATE INDEX routes_owner_idx ON routes(owner_user_id);
CREATE TABLE channel_grants (
    channel_id UUID NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    can_view BOOLEAN NOT NULL DEFAULT false,
    can_edit BOOLEAN NOT NULL DEFAULT false,
    can_send BOOLEAN NOT NULL DEFAULT false,
    PRIMARY KEY(channel_id,user_id),
    CHECK (can_view OR can_edit OR can_send)
);
CREATE INDEX channel_grants_user_idx ON channel_grants(user_id);
CREATE FUNCTION relay_channel_access(cid UUID, uid UUID, permission TEXT) RETURNS BOOLEAN
LANGUAGE SQL STABLE AS $$
    SELECT EXISTS(SELECT 1 FROM channels c JOIN users u ON u.id=uid AND u.enabled
      LEFT JOIN channel_grants g ON g.channel_id=c.id AND g.user_id=u.id
      WHERE c.id=cid AND (u.role='admin' OR c.owner_user_id=u.id OR
        CASE permission WHEN 'view' THEN COALESCE(g.can_view,false)
          WHEN 'edit' THEN COALESCE(g.can_edit,false)
          WHEN 'send' THEN COALESCE(g.can_send,false) ELSE false END))
$$;
CREATE FUNCTION relay_route_send(rid UUID, cid UUID) RETURNS BOOLEAN
LANGUAGE SQL STABLE AS $$
    SELECT EXISTS(SELECT 1 FROM routes r WHERE r.id=rid AND
      (r.owner_user_id IS NULL OR relay_channel_access(cid,r.owner_user_id,'send')))
$$;
-- Raw event data and routing reports can contain all recipients; require access to all.
CREATE FUNCTION relay_route_view(rid UUID, uid UUID) RETURNS BOOLEAN
LANGUAGE SQL STABLE AS $$
    SELECT EXISTS(SELECT 1 FROM routes r JOIN users u ON u.id=uid AND u.enabled
      WHERE r.id=rid AND (u.role='admin' OR (
        (r.owner_user_id=u.id OR EXISTS(SELECT 1 FROM route_channels rc WHERE rc.route_id=r.id))
        AND NOT EXISTS(SELECT 1 FROM (
          SELECT channel_id FROM route_channels WHERE route_id=r.id UNION
          SELECT channel_id FROM route_fallback_channels WHERE route_id=r.id UNION
          SELECT d.channel_id FROM deliveries d JOIN events e ON e.id=d.event_id WHERE e.route_id=r.id UNION
          SELECT f.channel_id FROM event_fallback_channels f JOIN events e ON e.id=f.event_id WHERE e.route_id=r.id
        ) targets WHERE NOT relay_channel_access(targets.channel_id,uid,'view')))))
$$;
