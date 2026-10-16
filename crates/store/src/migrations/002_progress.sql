ALTER TABLE downloads ADD COLUMN downloaded INTEGER NOT NULL DEFAULT 0;
ALTER TABLE downloads ADD COLUMN path TEXT;      -- where the finished file was saved
