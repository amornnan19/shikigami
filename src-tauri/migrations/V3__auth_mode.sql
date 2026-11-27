ALTER TABLE agents ADD COLUMN auth_mode TEXT
  CHECK (auth_mode IS NULL OR auth_mode IN ('subscription', 'api-key'));

INSERT INTO settings (key, value)
SELECT 'claude.auth-mode', CASE value WHEN 'true' THEN 'api-key' ELSE 'subscription' END
FROM settings
WHERE key = 'claude-code.use-api-key' AND value IN ('true', 'false');

DELETE FROM settings WHERE key = 'claude-code.use-api-key';
