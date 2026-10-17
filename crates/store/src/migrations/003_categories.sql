-- Whether a finished file is sorted into its category's folder.
ALTER TABLE downloads ADD COLUMN auto_category INTEGER NOT NULL DEFAULT 0;
