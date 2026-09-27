-- D1 schema of the feedback endpoint (services/feedback). Applied with
-- `wrangler d1 execute <db> --remote --file services/feedback/schema.sql`; every statement is
-- idempotent, so re-running it on a live database changes nothing.
CREATE TABLE IF NOT EXISTS feedback (
  id TEXT PRIMARY KEY,
  created_at INTEGER NOT NULL,
  kind TEXT NOT NULL,
  message TEXT NOT NULL,
  contact TEXT,
  diagnostics TEXT NOT NULL,
  ip_hash TEXT NOT NULL,
  country TEXT
);
CREATE INDEX IF NOT EXISTS feedback_by_client ON feedback (ip_hash, created_at);
CREATE INDEX IF NOT EXISTS feedback_by_time ON feedback (created_at);
