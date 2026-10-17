-- Full-text search over what a person remembers about a download: its name,
-- address and category. One row per download, rowid = downloads.id, kept in
-- step by triggers so no code path can forget it.
CREATE VIRTUAL TABLE downloads_fts USING fts5(
    filename, url, category,
    tokenize = 'unicode61 remove_diacritics 2'
);

INSERT INTO downloads_fts (rowid, filename, url, category)
SELECT d.id, COALESCE(d.filename, ''), d.url,
       COALESCE((SELECT name FROM categories WHERE id = d.category_id), '')
FROM downloads d;

CREATE TRIGGER downloads_fts_insert AFTER INSERT ON downloads BEGIN
    INSERT INTO downloads_fts (rowid, filename, url, category)
    VALUES (new.id, COALESCE(new.filename, ''), new.url,
            COALESCE((SELECT name FROM categories WHERE id = new.category_id), ''));
END;

CREATE TRIGGER downloads_fts_update
AFTER UPDATE OF filename, url, category_id ON downloads BEGIN
    DELETE FROM downloads_fts WHERE rowid = old.id;
    INSERT INTO downloads_fts (rowid, filename, url, category)
    VALUES (new.id, COALESCE(new.filename, ''), new.url,
            COALESCE((SELECT name FROM categories WHERE id = new.category_id), ''));
END;

CREATE TRIGGER downloads_fts_delete AFTER DELETE ON downloads BEGIN
    DELETE FROM downloads_fts WHERE rowid = old.id;
END;

CREATE INDEX downloads_created ON downloads(created_at);
