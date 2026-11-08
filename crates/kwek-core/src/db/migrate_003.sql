-- v3: search over conversation stretches. A stretch is a run of messages on one line of a
-- space (the main flow, or one thread's replies) without a long pause. Stretches are derived
-- data: a space whose messages change is marked dirty and its stretches are rebuilt.

ALTER TABLE space ADD COLUMN index_dirty INTEGER NOT NULL DEFAULT 1;

CREATE TABLE chunk (
  id       INTEGER PRIMARY KEY,
  space    TEXT NOT NULL,
  thread   TEXT,                -- set for a thread's replies; NULL for the main flow
  first_us INTEGER NOT NULL,
  last_us  INTEGER NOT NULL,
  msg_ids  TEXT NOT NULL,       -- JSON array of message.id, oldest first
  text     TEXT NOT NULL,       -- "Name: text" lines; keyword-indexed
  embed    TEXT NOT NULL,       -- the words only (no names, links shortened); embedded
  hash     TEXT NOT NULL        -- of embed; embeddings are keyed by it
);
CREATE INDEX chunk_space ON chunk(space);
-- Covering: the indexer's "newest without a vector" scan never reads the text pages.
CREATE INDEX chunk_hash ON chunk(hash, last_us);

CREATE VIRTUAL TABLE chunk_fts USING fts5(
  text,
  content='chunk', content_rowid='id',
  tokenize='unicode61 remove_diacritics 2'
);
CREATE TRIGGER chunk_fts_ai AFTER INSERT ON chunk BEGIN
  INSERT INTO chunk_fts(rowid, text) VALUES (new.id, new.text);
END;
CREATE TRIGGER chunk_fts_ad AFTER DELETE ON chunk BEGIN
  INSERT INTO chunk_fts(chunk_fts, rowid, text) VALUES ('delete', old.id, old.text);
END;

-- Vectors survive a rebuild that leaves a stretch's text unchanged.
CREATE TABLE embedding (
  hash  TEXT NOT NULL,
  model TEXT NOT NULL,
  vec   BLOB NOT NULL,          -- little-endian f32, L2-normalised
  PRIMARY KEY (hash, model)
);

CREATE TRIGGER message_dirty_ai AFTER INSERT ON message BEGIN
  UPDATE space SET index_dirty = 1 WHERE name = new.space;
END;
CREATE TRIGGER message_dirty_au AFTER UPDATE ON message
WHEN old.text IS NOT new.text OR old.delete_us IS NOT new.delete_us OR old.local_state IS NOT new.local_state
  OR old.thread IS NOT new.thread OR old.is_reply IS NOT new.is_reply OR old.sender IS NOT new.sender
BEGIN
  UPDATE space SET index_dirty = 1 WHERE name = new.space;
END;
CREATE TRIGGER message_dirty_ad AFTER DELETE ON message BEGIN
  UPDATE space SET index_dirty = 1 WHERE name = old.space;
END;
-- Stretch text carries sender names (display name, else email): only the spaces where this
-- person wrote need a rebuild.
CREATE INDEX message_sender ON message(sender);
CREATE TRIGGER user_dirty_ai AFTER INSERT ON user
WHEN new.display_name IS NOT NULL OR new.email IS NOT NULL BEGIN
  UPDATE space SET index_dirty = 1 WHERE name IN (SELECT DISTINCT space FROM message WHERE sender = new.name);
END;
CREATE TRIGGER user_dirty_au AFTER UPDATE OF display_name, email ON user
WHEN old.display_name IS NOT new.display_name OR old.email IS NOT new.email BEGIN
  UPDATE space SET index_dirty = 1 WHERE name IN (SELECT DISTINCT space FROM message WHERE sender = new.name);
END;
-- A space that goes away takes its stretches along.
CREATE TRIGGER space_chunks_ad AFTER DELETE ON space BEGIN
  DELETE FROM chunk WHERE space = old.name;
END;
