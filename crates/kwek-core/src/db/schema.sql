-- kwek schema v1. Times are integer microseconds since the epoch (see ts.rs).
-- Resource names are the keys; `raw` keeps the API JSON so new fields can be backfilled.

CREATE TABLE kv (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
);

CREATE TABLE account (
  id           TEXT PRIMARY KEY,          -- users/{id} of the signed-in user
  email        TEXT,
  display_name TEXT
);

CREATE TABLE space (
  name             TEXT PRIMARY KEY,      -- spaces/{id}
  space_type       TEXT NOT NULL,         -- SPACE | GROUP_CHAT | DIRECT_MESSAGE
  display_name     TEXT,
  threaded         INTEGER NOT NULL DEFAULT 0,
  last_active_us   INTEGER,
  last_synced_us   INTEGER,               -- reconcile high-water mark
  member_count     INTEGER,
  space_uri        TEXT,
  history_complete INTEGER NOT NULL DEFAULT 0,  -- oldest message is cached
  dm_peer          TEXT,                  -- DIRECT_MESSAGE: the other user
  muted            INTEGER NOT NULL DEFAULT 0,  -- local only
  starred          INTEGER NOT NULL DEFAULT 0,  -- local only
  raw              TEXT NOT NULL
);

CREATE TABLE user (
  name           TEXT PRIMARY KEY,        -- users/{id}
  display_name   TEXT,
  email          TEXT,
  avatar_url     TEXT,
  resolved_at_us INTEGER                  -- People API lookup; NULL = not looked up yet
);

CREATE TABLE membership (
  space  TEXT NOT NULL,
  member TEXT NOT NULL,
  role   TEXT,
  PRIMARY KEY (space, member)
);

-- `id` aliases the rowid explicitly so it never changes: the FTS index is keyed by it, and
-- writes use INSERT ... ON CONFLICT(name) DO UPDATE (never INSERT OR REPLACE).
CREATE TABLE message (
  id             INTEGER PRIMARY KEY,
  name           TEXT NOT NULL UNIQUE,    -- spaces/{s}/messages/{m}; pending:{client_id} before the server answers
  space          TEXT NOT NULL,
  thread         TEXT,
  is_reply       INTEGER NOT NULL DEFAULT 0,
  sender         TEXT NOT NULL,
  create_us      INTEGER NOT NULL,
  update_us      INTEGER NOT NULL,        -- lastUpdateTime, else createTime: upsert ordering
  delete_us      INTEGER,                 -- tombstone; once set, the message stays deleted
  text           TEXT,                    -- plain text, indexed for search
  formatted_text TEXT,
  annotations    TEXT,                    -- JSON (rich links, mentions)
  attachments    TEXT,                    -- JSON
  cards          TEXT,                    -- JSON
  reactions      TEXT,                    -- JSON emojiReactionSummaries
  mentions_me    INTEGER NOT NULL DEFAULT 0,
  client_id      TEXT,                    -- clientAssignedMessageId of optimistic sends
  local_state    TEXT NOT NULL DEFAULT 'synced',  -- synced | pending | failed | ghost
  raw            TEXT NOT NULL
);
CREATE INDEX message_space_time ON message(space, create_us);
CREATE INDEX message_thread_time ON message(thread, create_us);
CREATE INDEX message_client ON message(client_id) WHERE client_id IS NOT NULL;

CREATE TABLE my_reaction (
  message       TEXT NOT NULL,
  emoji         TEXT NOT NULL,            -- Emoji::key()
  reaction_name TEXT NOT NULL,            -- for delete; 'pending' while optimistic
  PRIMARY KEY (message, emoji)
);

CREATE TABLE space_read_state (
  space        TEXT PRIMARY KEY,
  last_read_us INTEGER
);

CREATE TABLE thread_read_state (
  thread       TEXT PRIMARY KEY,
  last_read_us INTEGER
);

CREATE TABLE media_cache (
  resource     TEXT PRIMARY KEY,
  path         TEXT NOT NULL,
  mime         TEXT,
  bytes        INTEGER,
  last_used_us INTEGER
);

-- External-content FTS over message.text, so snippet() works. Kept in sync by triggers.
CREATE VIRTUAL TABLE message_fts USING fts5(
  text,
  content='message', content_rowid='id',
  tokenize='unicode61 remove_diacritics 2'
);

CREATE TRIGGER message_fts_ai AFTER INSERT ON message BEGIN
  INSERT INTO message_fts(rowid, text) VALUES (new.id, new.text);
END;
CREATE TRIGGER message_fts_ad AFTER DELETE ON message BEGIN
  INSERT INTO message_fts(message_fts, rowid, text) VALUES ('delete', old.id, old.text);
END;
CREATE TRIGGER message_fts_au AFTER UPDATE OF text ON message BEGIN
  INSERT INTO message_fts(message_fts, rowid, text) VALUES ('delete', old.id, old.text);
  INSERT INTO message_fts(rowid, text) VALUES (new.id, new.text);
END;
