-- Generation-aware resume checkpoints for native sources. A checkpoint names
-- the source generation it belongs to (a device/inode identity plus a digest of
-- the bytes ending at the position for a transcript file; a scan instant for a
-- reader host) so a later scan can prove the input before it may be skipped.
-- The zero-position rows in source_cursors stay plain locators; this table is
-- the only resume authority, and a checkpoint may move backwards when a source
-- is replaced or truncated.
CREATE TABLE native_checkpoints (
    source TEXT NOT NULL CHECK(source IN ('transcript', 'readers_cli')),
    cursor_key TEXT NOT NULL CHECK(length(trim(cursor_key)) > 0),
    generation TEXT NOT NULL CHECK(json_valid(generation) AND json_type(generation) = 'object'),
    position INTEGER NOT NULL CHECK(position >= 0),
    updated_at INTEGER NOT NULL,
    PRIMARY KEY(source, cursor_key)
);
