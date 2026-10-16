CREATE TABLE queues (
    id             INTEGER PRIMARY KEY,
    name           TEXT    NOT NULL UNIQUE,
    max_active     INTEGER NOT NULL DEFAULT 3 CHECK (max_active >= 1),
    speed_limit    INTEGER,              -- bytes per second, NULL = unlimited
    priority       INTEGER NOT NULL DEFAULT 0,
    schedule_json  TEXT                  -- NULL = always on
);

CREATE TABLE categories (
    id          INTEGER PRIMARY KEY,
    name        TEXT NOT NULL UNIQUE,
    extensions  TEXT NOT NULL DEFAULT '', -- comma separated, lower case, no dots
    folder      TEXT                      -- NULL = the default download folder
);

CREATE TABLE downloads (
    id           INTEGER PRIMARY KEY,
    url          TEXT    NOT NULL,
    final_url    TEXT,
    filename     TEXT,
    dest_dir     TEXT    NOT NULL,
    size         INTEGER,
    status       TEXT    NOT NULL DEFAULT 'queued'
                 CHECK (status IN ('queued','probing','downloading','paused',
                                   'retrying','completed','failed','canceled')),
    error        TEXT,
    category_id  INTEGER REFERENCES categories(id) ON DELETE SET NULL,
    queue_id     INTEGER NOT NULL REFERENCES queues(id),
    position     INTEGER NOT NULL DEFAULT 0,
    speed_limit  INTEGER,
    headers_json TEXT,
    etag         TEXT,
    last_modified TEXT,
    created_at   INTEGER NOT NULL,       -- unix seconds
    finished_at  INTEGER
);
CREATE INDEX downloads_status ON downloads(status);
CREATE INDEX downloads_queue  ON downloads(queue_id, position);

CREATE TABLE segments (
    download_id INTEGER NOT NULL REFERENCES downloads(id) ON DELETE CASCADE,
    start       INTEGER NOT NULL,
    committed   INTEGER NOT NULL,        -- next byte not yet on disk
    "end"       INTEGER NOT NULL,        -- exclusive
    PRIMARY KEY (download_id, start)
);

CREATE TABLE settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE log (
    id          INTEGER PRIMARY KEY,
    download_id INTEGER REFERENCES downloads(id) ON DELETE CASCADE,
    at          INTEGER NOT NULL,
    level       TEXT NOT NULL,
    message     TEXT NOT NULL
);
CREATE INDEX log_download ON log(download_id, at);

INSERT INTO queues (name, max_active) VALUES ('Main', 3);
INSERT INTO categories (name, extensions) VALUES
 ('Compressed','zip,rar,7z,gz,tar,bz2,xz,zst'),
 ('Documents','pdf,doc,docx,xls,xlsx,ppt,pptx,txt,epub,odt'),
 ('Music','mp3,flac,wav,aac,ogg,m4a,opus'),
 ('Programs','exe,msi,dmg,pkg,deb,rpm,apk,appimage'),
 ('Video','mp4,mkv,avi,mov,webm,flv,wmv,m4v');
