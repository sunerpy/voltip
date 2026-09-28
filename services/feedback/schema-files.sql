-- D1 schema of the feedback endpoint's files database (services/feedback): the attachments a
-- report declares (screenshots, screen recordings) and their bytes in 1 MiB chunks. A database of
-- its own, so the reports never share a size limit with the bytes. Applied with
-- `wrangler d1 execute <files-db> --remote --file services/feedback/schema-files.sql`; every
-- statement is idempotent.
CREATE TABLE IF NOT EXISTS attachment (
  report_id TEXT NOT NULL,
  idx INTEGER NOT NULL,
  name TEXT NOT NULL,
  type TEXT NOT NULL,
  size INTEGER NOT NULL,
  sha256 TEXT NOT NULL,
  token_hash TEXT NOT NULL,
  ip_hash TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  uploaded_at INTEGER,
  PRIMARY KEY (report_id, idx)
);
CREATE INDEX IF NOT EXISTS attachment_by_client ON attachment (ip_hash, created_at);
CREATE TABLE IF NOT EXISTS attachment_chunk (
  report_id TEXT NOT NULL,
  idx INTEGER NOT NULL,
  seq INTEGER NOT NULL,
  data BLOB NOT NULL,
  PRIMARY KEY (report_id, idx, seq)
);
