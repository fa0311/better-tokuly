-- Media: uploaded assets and their processed renditions.
--
-- Object layout (see crates/media/src/store.rs):
--   private bucket  originals/<asset_id>/source[.ext]      the upload (what the upload URLs write)
--                   originals/<asset_id>/stored-<n>[.ext]  a completed single-PUT upload, out of
--                                                          reach of its still valid upload URL
--   media bucket    img/<asset_id>/<size>-<hash>.webp      image variants
--                   anim/<asset_id>/<hash>.webp            animated WebP
--                   video/<asset_id>/...                   HLS ladder, MP4, storyboard, preview
--                   audio/<asset_id>/...                   voice message renditions
--                   file/<asset_id>/<hash>.<ext>           public files (dotLottie stamps)

CREATE TABLE media_assets (
    id            uuid PRIMARY KEY DEFAULT uuidv7(),
    owner_id      uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- tokuly.media.v1.AssetPurpose / AssetKind / AssetStatus
    purpose       integer     NOT NULL,
    -- Declared kind while UPLOADING; the detected kind afterwards.
    kind          integer     NOT NULL,
    status        integer     NOT NULL,
    -- Declared by the client while UPLOADING; detected from the bytes afterwards.
    content_type  text        NOT NULL DEFAULT '',
    size_bytes    bigint      NOT NULL DEFAULT 0,
    file_name     text        NOT NULL DEFAULT '',
    -- Lowercase hex SHA-256 the client promised; '' when none was given.
    sha256        text        NOT NULL DEFAULT '',

    -- Where the original bytes live in the private bucket (the upload key,
    -- or the stored copy of a completed single-PUT upload).
    original_key  text        NOT NULL,
    -- Storage multipart upload ID while a multipart upload is in flight.
    multipart_id  text,
    part_size     bigint      NOT NULL DEFAULT 0,
    part_count    integer     NOT NULL DEFAULT 0,

    -- Processing: 0-100 while PROCESSING, the machine-readable reason when FAILED.
    progress      integer     NOT NULL DEFAULT 0,
    error         text        NOT NULL DEFAULT '',
    -- Times a worker started processing; bounds retries of a crashing input.
    attempts      integer     NOT NULL DEFAULT 0,
    -- Set (and refreshed every few seconds) by the worker processing the
    -- asset, cleared when it gives the asset back for a retry. A fresh value
    -- means "someone is on it": duplicate jobs skip the asset, and the
    -- janitor re-queues assets whose worker went silent.
    heartbeat_at  timestamptz,

    -- Results of processing. `url` stays '' for private files, whose URL is
    -- presigned on every read.
    url           text        NOT NULL DEFAULT '',
    -- tokuly.common.v1.Image as proto JSON.
    image         jsonb,
    -- tokuly.media.v1.VideoInfo as proto JSON.
    video         jsonb,
    waveform      bytea,

    -- Usage tracking (see media_asset_holders). `holder_count` mirrors the
    -- number of holder rows so that "is it in use?" is a property of this
    -- row and can be checked atomically by the statement that deletes it.
    -- `detached_at` is when the last holder let go; an asset nobody holds is
    -- collected some time after that (or after `ready_at` if never held).
    holder_count  integer     NOT NULL DEFAULT 0 CHECK (holder_count >= 0),
    detached_at   timestamptz,

    attrs         jsonb       NOT NULL DEFAULT '{}',
    created_at    timestamptz NOT NULL DEFAULT now(),
    updated_at    timestamptz NOT NULL DEFAULT now(),
    ready_at      timestamptz,
    -- Soft delete: the row disappears for readers at once, the janitor then
    -- removes the stored objects and stamps `purged_at`. The row itself stays
    -- as a tombstone so other domains' references never dangle.
    deleted_at    timestamptz,
    purged_at     timestamptz,
    -- When a presigned upload URL may have outlived the asset's upload: a
    -- late PUT can still store bytes under the upload key. The janitor then
    -- runs once more after this time: a deleted asset is purged again, a
    -- completed one loses whatever was stored under its upload key.
    repurge_after timestamptz
);

CREATE INDEX media_assets_owner ON media_assets (owner_id, id DESC);
-- CreateUpload's limits: an owner's unfinished uploads and unused bytes.
CREATE INDEX media_assets_owner_unused ON media_assets (owner_id)
    WHERE deleted_at IS NULL AND status IN (1, 2, 3) AND (holder_count = 0 OR status = 1);
-- CreateUpload's daily byte budget: what an owner uploaded in the last 24 hours.
CREATE INDEX media_assets_owner_created ON media_assets (owner_id, created_at);
-- Janitor scans: abandoned uploads, stalled jobs, objects awaiting removal.
CREATE INDEX media_assets_stale_uploads ON media_assets (created_at) WHERE status = 1 AND deleted_at IS NULL;
CREATE INDEX media_assets_processing ON media_assets (updated_at) WHERE status = 2 AND deleted_at IS NULL;
CREATE INDEX media_assets_unpurged ON media_assets (deleted_at) WHERE deleted_at IS NOT NULL AND purged_at IS NULL;
CREATE INDEX media_assets_repurge ON media_assets (repurge_after) WHERE repurge_after IS NOT NULL;
CREATE INDEX media_assets_failed ON media_assets (updated_at) WHERE status = 4 AND deleted_at IS NULL;
CREATE INDEX media_assets_unattached ON media_assets (ready_at) WHERE status = 3 AND deleted_at IS NULL AND holder_count = 0;

CREATE TRIGGER media_assets_updated_at
    BEFORE UPDATE ON media_assets
    FOR EACH ROW
EXECUTE FUNCTION set_updated_at();

-- Who uses an asset. A holder is a free-form reference chosen by the using
-- domain ("identity:avatar:<user_id>", "stamp:<stamp_id>", "chat:message:<id>").
-- While an asset has holders its owner cannot delete it; once it has none
-- for long enough the janitor collects it. Written only through
-- tokuly_media::assets::{attach, detach, detach_holder}.
CREATE TABLE media_asset_holders (
    asset_id   uuid        NOT NULL REFERENCES media_assets (id) ON DELETE CASCADE,
    holder     text        NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (asset_id, holder)
);

CREATE INDEX media_asset_holders_holder ON media_asset_holders (holder);

-- Cached Open Graph lookups for link previews.
CREATE TABLE link_previews (
    -- Hex SHA-256 of the normalized URL.
    url_hash   text PRIMARY KEY,
    url        text        NOT NULL,
    -- tokuly.media.v1.LinkPreview as proto JSON; NULL when the page had
    -- nothing useful or could not be fetched (negative cache).
    preview    jsonb,
    fetched_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX link_previews_fetched_at ON link_previews (fetched_at);

-- Pictures of link previews, re-encoded and kept for a day
-- (link_preview::image_proxy). The WebP itself is the object
-- `link-images/<cache_key>.webp` of the private bucket; the janitor removes
-- expired rows together with their objects.
CREATE TABLE link_preview_images (
    -- `<kind>/<hex SHA-256 of the picture's URL>`.
    cache_key  text PRIMARY KEY,
    -- false: there was nothing to show (negative cache, no object).
    found      boolean     NOT NULL,
    size_bytes integer     NOT NULL DEFAULT 0,
    fetched_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX link_preview_images_fetched_at ON link_preview_images (fetched_at);
