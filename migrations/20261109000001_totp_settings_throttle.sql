-- Security-settings failures belong to the current session, not the sign-in factor.
ALTER TABLE sessions ADD COLUMN totp_settings_failures INT NOT NULL DEFAULT 0;
ALTER TABLE sessions ADD COLUMN totp_settings_locked_until TIMESTAMPTZ;
