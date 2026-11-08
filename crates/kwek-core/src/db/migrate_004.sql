-- Sidebar preferences, local only like muted and starred: a conversation hidden from the
-- sidebar's main sections, and the user's own order for the Spaces section.
ALTER TABLE space ADD COLUMN hidden INTEGER NOT NULL DEFAULT 0;
-- Position in the custom order; NULL for spaces never placed (they follow the placed ones).
ALTER TABLE space ADD COLUMN position INTEGER;
