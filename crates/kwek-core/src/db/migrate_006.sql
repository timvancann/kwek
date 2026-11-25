-- v6: the message a message quotes (JSON quotedMessageMetadata).
ALTER TABLE message ADD COLUMN quote TEXT;

-- Messages cached before this were stored without the field (raw is the parsed message, not
-- Google's JSON), so download every space's history again in the background to fill it in.
UPDATE space SET low_water_us = NULL, history_complete = 0;
