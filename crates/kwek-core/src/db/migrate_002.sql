-- v2: contiguous history low-water mark per space; the media cache is the hashed file on
-- disk, so its table goes.

-- Oldest createTime of the contiguous cached history that reaches the present. Only an
-- initial fetch or paging back moves it; stray old messages (reaction refetches, search
-- hits) do not, so scroll-up never skips a gap. NULL: nothing fetched yet.
ALTER TABLE space ADD COLUMN low_water_us INTEGER;

UPDATE space SET low_water_us = (
  SELECT min(create_us) FROM message m WHERE m.space = space.name AND m.local_state = 'synced' AND m.create_us > 0
);

DROP TABLE media_cache;
