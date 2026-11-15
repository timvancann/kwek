-- Local only, like muted and starred: messages the user saved, reminders about messages, and
-- messages written now to send later. Google Chat's API has none of these.
CREATE TABLE saved (
  message  TEXT PRIMARY KEY,               -- spaces/{s}/messages/{m}
  saved_us INTEGER NOT NULL
);

CREATE TABLE reminder (
  message TEXT PRIMARY KEY,
  at_us   INTEGER NOT NULL
);
CREATE INDEX reminder_at ON reminder(at_us);

-- Sent by kwek at `at_us`, so only while it runs (or as soon as it starts again).
CREATE TABLE scheduled (
  id     INTEGER PRIMARY KEY,
  space  TEXT NOT NULL,
  thread TEXT,
  text   TEXT NOT NULL,
  files  TEXT NOT NULL DEFAULT '[]',       -- JSON: the staged outbox files (sync::Outgoing)
  at_us  INTEGER NOT NULL
);
CREATE INDEX scheduled_at ON scheduled(at_us);
