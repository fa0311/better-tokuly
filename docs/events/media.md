# media

Uploads, assets, image/video/audio processing and link previews
(`backend/crates/media`, `proto/tokuly/media/v1/media.proto`,
`backend/migrations/0400_media.sql`).

## Events emitted

Payload struct: `tokuly_media::events::AssetEvent`
(`{ asset_id, owner_id, purpose, kind[, error] }`; `purpose` and `kind` are the
numbers of `AssetPurpose` / `AssetKind`). The actor is the asset's owner.

| Topic | When | Notes |
|---|---|---|
| `media.asset.ready` | processing finished, the asset is READY | emitted in the transaction that makes it READY |
| `media.asset.failed` | the upload was rejected or could not be processed | `error` holds the reason (`UNSUPPORTED_CONTENT`, `CORRUPT_MEDIA`, `NOT_A_VIDEO`, `NO_AUDIO`, `IMAGE_TOO_LARGE`, `ANIMATION_TOO_LARGE`, `VIDEO_TOO_LARGE`, `MEDIA_TOO_LONG`, `DAILY_TRANSCODE_LIMIT_EXCEEDED`, `SIZE_MISMATCH`, `CHECKSUM_MISMATCH`, `PROCESSING_FAILED`) |
| `media.asset.deleted` | the owner, another domain (`assets::delete`) or the janitor removed a completed asset | not emitted for uploads aborted before completion (unless something held them), nor for the bulk removal of a deleted account's unheld assets |

Domains that hold a not-yet-ready asset (a video being published, a message
whose attachment is still transcoding) should consume `media.asset.ready` /
`media.asset.failed` to move their own state forward.

## Events consumed

| Topic | Durable | Effect |
|---|---|---|
| `identity.user.deleted` | `media-user-deleted` | the user's assets that nothing holds are deleted at once (objects removed); held ones stay until their holders detach, then the janitor collects them |

## Usage tracking (holders)

What keeps an asset alive is being *held*. A domain that stores a reference
to an asset records a holder for it, in the same transaction, and releases
it when the reference goes away (`tokuly_media::assets`, table
`media_asset_holders`):

```rust
assets::attach(&mut *tx, asset_id, "chat:message:<id>").await?;   // idempotent
assets::detach(&mut *tx, asset_id, "identity:avatar:<user_id>").await?;
assets::detach_holder(&mut *tx, "notes:note:<id>").await?;       // everything that holder held
assets::is_attached(core, asset_id).await?;
```

A holder is a free-form string chosen by the using domain; the convention is
`<domain>:<entity>:<id>`. The rules:

- an owner cannot delete a held asset: `DeleteAsset`, `AbortUpload` and
  `assets::delete(core, id, Some(owner))` fail with `ASSET_IN_USE`
  (failed precondition);
- `assets::delete(core, id, None)` (a domain acting on its own authority,
  e.g. a takedown) deletes regardless and drops the holds; holders learn of
  it through `media.asset.deleted`;
- a READY asset with no holder for `TOKULY_MEDIA_UNATTACHED_TTL_HOURS` is
  deleted by the janitor — counted from when it became ready, or from when
  its last holder detached;
- account deletion removes only the account's unheld assets.

**Every domain that keeps an asset's URLs must attach it**, or those URLs
die when the janitor collects the asset (48 hours after upload by default).
Set `TOKULY_MEDIA_UNATTACHED_TTL_HOURS=0` to switch collection off.

## Realtime

`MediaEvent.asset_updated` carries the whole `Asset` on
every progress step and state change:

- to the owner (all devices), in the owner's view;
- to the topic `asset:{asset_id}`, in the public view (no file name, no URL of
  private files). Anyone who knows the ID may subscribe, e.g. the recipient
  of a message whose attachment is still processing.

`MediaService.WatchAsset` is the same information as a server stream for the
owner.

## Notification kinds

None. Domains that own the content (video, chat…) notify about it.

## Background jobs

| Job | Payload | Concurrency | What |
|---|---|---|---|
| `media.process` | `ProcessJob { asset_id }` | `TOKULY_MEDIA_IMAGE_CONCURRENCY` | images, animations, voice messages, deferred file checks |
| `media.transcode` | `ProcessJob { asset_id }` | `TOKULY_MEDIA_VIDEO_CONCURRENCY` | video (a turn on the fair queue, below) |

A job claims its asset (`media_assets.heartbeat_at`), so duplicates are
no-ops. Infrastructure failures are retried up to 3 attempts per asset, then
the asset fails with `PROCESSING_FAILED`; a rejected input fails at once.

### How long a recording may be

A recording longer than its purpose allows fails with `MEDIA_TOO_LONG`
before anything is transcoded (1 s of slack for recorders that stop a few
frames late). The length is measured from the packets, not taken from the
container header, so a file that understates or hides its length does not
get through; the daily transcoding budget is charged with the measured
length. The limit is in `CreateUploadResponse.max_duration_seconds` and
`Asset.max_duration_seconds`, so clients can say what it is.

| Purpose | Longest |
|---|---|
| `CHAT_AUDIO` (voice messages) | 5 min |
| `CHAT_VIDEO`, `NOTE_VIDEO` | 10 min |
| `STORY` | 60 s |
| `SHORT` (live clips too: at most 60 s already) | 3 min (shorts checks its own `TOKULY_SHORTS_MAX_DURATION_SECONDS` on publishing) |
| `AVATAR_VIDEO` | 15 s |
| `VIDEO` | `TOKULY_MEDIA_MAX_VIDEO_HOURS` (3 h) |
| `LIVE_ARCHIVE` | none here: live ends streams after `TOKULY_LIVE_MAX_HOURS` |

### Pictures by purpose

Every picture becomes WebP variants of 2048/1024/512/256/96 px (longest
edge, never upscaled). A purpose that is only ever shown small gets no
variant above what it is shown at, and animations are kept only where they
are wanted:

| Purpose | Largest variant | Animation | Size limit |
|---|---|---|---|
| `COMMENT_IMAGE` (a picture in a comment) | 1024 px | kept | `TOKULY_MEDIA_MAX_COMMENT_IMAGE_MB` |
| `CUSTOM_EMOJI` | 256 px (animated rendition: 256 px too) | kept | `TOKULY_MEDIA_MAX_EMOJI_MB` |
| `COMMUNITY_ICON` | 512 px | kept | `TOKULY_MEDIA_MAX_AVATAR_MB` |
| `COMMUNITY_BANNER` | 2048 px | first frame | `TOKULY_MEDIA_MAX_AVATAR_MB` |
| every other picture | 2048 px | kept for `AVATAR`, `STAMP`, `CHAT_IMAGE`, `NOTE_IMAGE`, `STORY` (animated rendition: 512 px) | by purpose (Configuration) |

Domains also accept uploads made with the purposes these four replaced
(`NOTE_IMAGE` / `CHAT_IMAGE` for comments, `STAMP` for emoji, `AVATAR` /
`CHANNEL_ICON` and `BANNER` / `CHANNEL_BANNER` for communities).

### The fair transcoding queue

Owners take turns on the transcoder: a `media.transcode` job transcodes the
waiting video that is fairest to go next (each owner's oldest, owners with
fewer videos running and a longer wait first), not necessarily its own. An
owner with `TOKULY_MEDIA_TRANSCODES_PER_ACCOUNT` (2) transcodes running gets
no more until one ends. So one account's backlog never holds up anyone
else's first video. `Media::transcode_next` takes one turn in place (tests,
tooling); `process_now` processes the named asset.

On top of the daily byte budget, an account may have
`TOKULY_MEDIA_DAILY_TRANSCODE_MINUTES` (720) minutes of video transcoded in
any 24 hours. A video is booked once, when probed (also when it fails or is
deleted later); one that does not fit fails with
`DAILY_TRANSCODE_LIMIT_EXCEEDED`. Media the server makes itself (live
archives and clips) is not booked.

### The janitor

Every 5 minutes (worker processes): aborts uploads unfinished after
`TOKULY_MEDIA_UPLOAD_TTL_HOURS`, removes unheld FAILED assets after 7 days,
deletes READY assets that nothing has held for
`TOKULY_MEDIA_UNATTACHED_TTL_HOURS`, purges objects of deleted assets,
re-queues PROCESSING assets whose worker died, and drops expired budget
rows, link preview cache rows (30 days) and cached preview pictures (a day).

## Link previews

- **This instance's own pages** are never fetched: the preview is
  `embed_provider = "tokuly"` with `embed_id = <kind>/<id>` (see
  `LinkPreview` in the proto). A preview is stored once and shown to
  everyone in the conversation, so the client asks the owning domain with
  the viewer's own permissions: content the viewer may not read previews as
  a plain link.
- **Pictures and site icons** (`image_url`, `favicon_url`) never point at
  the other site, so viewing a preview tells that site nothing. Every
  preview handed out points them at
  `{TOKULY_PUBLIC_URL}{TOKULY_API_PATH_PREFIX}/media/link-image/{image|icon}/{signature}/{base64url of the URL}`,
  and the API fetches them on the viewer's behalf.
- The route answers only URLs the server signed (HMAC under
  `TOKULY_SECRET`; anything else is a 404), fetches through
  `tokuly_core::egress` (5 s, 8 MB), and serves only raster images (never
  SVG) re-encoded as WebP of at most 1200 px (pictures) or 128 px (icons).
  Results are cached for 24 hours ("nothing to show" for 30 minutes);
  responses carry `Cache-Control: public, max-age=86400`.
- Misses are bounded: 4 downloads at once per API process (beyond that
  `503` with `Retry-After: 60`), 60 picture fetches a minute per site and
  120 misses a minute per client address (per /64 for IPv6).
- A picture on this instance's media origin (`TOKULY_MEDIA_URL`) is not
  proxied: the preview keeps its URL, so it ends the moment the file is
  deleted. Any other URL of this instance is dropped.
- Clients show a preview picture only when it is a proxy URL or one of this
  instance's own `/media/` files.

## Configuration (`MediaConfig`)

| Variable | Default | Meaning |
|---|---|---|
| `TOKULY_MEDIA_MAX_AVATAR_MB` | 10 | avatars, banners, channel art, community icons and banners, thumbnails, stamps |
| `TOKULY_MEDIA_MAX_IMAGE_MB` | 25 | photos in chats, notes, stories, places, day memos |
| `TOKULY_MEDIA_MAX_COMMENT_IMAGE_MB` | 10 | pictures in comments (`COMMENT_IMAGE`) |
| `TOKULY_MEDIA_MAX_EMOJI_MB` | 2 | custom emoji pictures (`CUSTOM_EMOJI`); the emoji domain accepts less (`TOKULY_EMOJI_MAX_IMAGE_KB`, 1024), keep this one above it |
| `TOKULY_MEDIA_MAX_CLIP_MB` | 200 | chat/note videos, story videos, avatar videos |
| `TOKULY_MEDIA_MAX_VIDEO_MB` | 20480 | long-form video |
| `TOKULY_MEDIA_MAX_SHORT_MB` | 1024 | shorts |
| `TOKULY_MEDIA_MAX_AUDIO_MB` | 25 | voice messages |
| `TOKULY_MEDIA_MAX_FILE_MB` | 100 | chat file attachments (subtitles and dotLottie stamps: 2 MB, fixed) |
| `TOKULY_MEDIA_MULTIPART_THRESHOLD_MB` | 16 | larger uploads are multipart |
| `TOKULY_MEDIA_PART_SIZE_MB` | 16 | multipart part size (≥ 5; grows automatically past 10 000 parts) |
| `TOKULY_MEDIA_UPLOAD_TTL_HOURS` | 24 | unfinished uploads are discarded after this |
| `TOKULY_MEDIA_UNATTACHED_TTL_HOURS` | 48 | finished uploads nothing holds are deleted after this (0 = never) |
| `TOKULY_MEDIA_MAX_UNUSED_GB` | 100 | per account: bytes of uploads nothing holds yet (unfinished, processing, ready but unattached); `CreateUpload` beyond it fails with `UNUSED_QUOTA_EXCEEDED` |
| `TOKULY_MEDIA_DAILY_UPLOAD_GB` | 50 | per account: bytes uploaded in any 24 hours, used or not (uploads aborted before completion do not count); beyond it `DAILY_UPLOAD_LIMIT_EXCEEDED` |
| `TOKULY_MEDIA_IMAGE_CONCURRENCY` | 4 | parallel `media.process` jobs per worker process |
| `TOKULY_MEDIA_VIDEO_CONCURRENCY` | 1 | parallel `media.transcode` jobs per worker process |
| `TOKULY_MEDIA_FFMPEG_THREADS` | 0 | threads per ffmpeg process (0 = ffmpeg decides) |
| `TOKULY_MEDIA_X264_PRESET` | `veryfast` | x264 preset of the HLS ladder |
| `TOKULY_MEDIA_AV1` | `false` | add an AV1 (SVT-AV1) rendition of the top rung |
| `TOKULY_MEDIA_MAX_VIDEO_HOURS` | 3 | longest long-form video (`ASSET_PURPOSE_VIDEO`); other purposes have fixed limits (above) |
| `TOKULY_MEDIA_TRANSCODES_PER_ACCOUNT` | 2 | videos of one account transcoded at once, across all workers |
| `TOKULY_MEDIA_DAILY_TRANSCODE_MINUTES` | 720 | per account: minutes of video transcoded in any 24 hours (0 = no budget); beyond it `DAILY_TRANSCODE_LIMIT_EXCEEDED` |
| `TOKULY_MEDIA_FFMPEG_PATH` / `TOKULY_MEDIA_FFPROBE_PATH` | `ffmpeg` / `ffprobe` | binaries |
| `TOKULY_MEDIA_WORK_DIR` | system temp | scratch space for transcodes |

## Files are kept as they are

Images, animations, video and audio are decoded and re-encoded: EXIF, GPS
positions, device names and container metadata are gone. Files
(`ASSET_KIND_FILE`: chat files, subtitles, dotLottie stamps) are stored and
served byte for byte, **metadata included**: a photo sent as a chat file
keeps its location. Clients therefore send photos as images, and convert
formats the image pipeline does not decode (HEIC, HEIF, AVIF, BMP; it
decodes JPEG, PNG, WebP and GIF) to JPEG before upload instead of falling
back to a file.

## Object layout

Private bucket (presigned access only): `originals/{asset_id}/source[.ext]`
is what the upload URLs write. A completed single-PUT upload is copied to
`originals/{asset_id}/stored-{n}[.ext]` before its bytes are checked, so the
upload URL (valid for an hour) cannot replace a finished file.

Media bucket (public, `Cache-Control: public, max-age=300, s-maxage=0, must-revalidate`; see "Caches and CDNs"):

```
img/{asset_id}/{size}-{hash}.webp          image variants (2048/1024/512/256/96 px longest edge, never upscaled; fewer for purposes shown small)
anim/{asset_id}/{hash}.webp                animated WebP of animated uploads
video/{asset_id}/hls/master.m3u8           adaptive playlist
video/{asset_id}/hls/{rung}/…              H.264 rungs (1080p/720p/480p/360p/240p by short side), fMP4 segments of 4 s
video/{asset_id}/hls/{rung}-av1/…          optional AV1 rendition
video/{asset_id}/hls/audio/…               AAC audio shared by all rungs
video/{asset_id}/storyboard/storyboard.vtt WebVTT thumbnail track + sprite_NNN.webp sheets
video/{asset_id}/preview.webp              3 s muted animated preview
video/{asset_id}/clip.mp4                  fast-start MP4 (clips ≤ 60 s)
audio/{asset_id}/voice.m4a, voice.webm     voice message renditions (AAC, Opus)
file/{asset_id}/{hash}.lottie              dotLottie stamps
```

## Object store requirements for browser uploads

Browsers PUT straight to the private bucket with presigned URLs, so the
bucket needs a CORS rule for the web origin(s):

- `AllowedMethods`: `PUT` (and `GET`/`HEAD` for downloads of private files);
- `AllowedHeaders`: `Content-Type` (single PUT; the value is signed — send
  exactly `put_headers`), or `*`;
- `ExposeHeaders`: `ETag` (optional but recommended: lets the client pass
  `CompletedPart.etag` so the server can cross-check parts; uploads complete
  without it because the server lists the stored parts itself).

`Content-Length` is part of every upload signature: a URL only accepts a
body of exactly the announced size (browsers set the header from the body,
nothing to do client-side). Presigned URLs are signed for
`TOKULY_S3_PUBLIC_ENDPOINT`, which must be the host clients reach. The media
bucket is served through the edge at `TOKULY_MEDIA_URL`; players need CORS
`GET` there too when the app and media origins differ (HLS via MSE, WebVTT
tracks).

## Caches and CDNs

Keys in the media bucket never change their content, but any of them can be
**removed** (owner, moderator, janitor), and a removed object must stop
being served at once. So nothing is cached as immutable:

- `assets::delete` commits the deletion, then purges the objects (the
  janitor retries within five minutes if the object store fails).
- The edge (`infra/caddy/Caddyfile`) serves `/media/*` with
  `Cache-Control: public, max-age=300, s-maxage=0, must-revalidate`: shared
  caches revalidate every request, browsers keep a copy for at most five
  minutes.
- The edge tags every object of an asset with `Cache-Tag: asset-<asset_id>`
  and `Surrogate-Key: asset-<asset_id>`.

A CDN that honours these headers needs nothing else. One configured to cache
`/media/*` for longer — or one that pulls straight from the bucket and does
not apply the header above — keeps serving removed media and **must purge on
deletion**:

1. Consume `media.asset.deleted` from the `EVENTS` JetStream stream (subject
   `evt.media.asset.deleted`, prefixed with `TOKULY_NATS_PREFIX.` when set)
   with a durable consumer of its own.
2. Purge by tag `asset-<asset_id>`, or by prefix under
   `{TOKULY_MEDIA_URL}/img/<asset_id>/`, `/anim/<asset_id>/`,
   `/video/<asset_id>/`, `/audio/<asset_id>/` and `/file/<asset_id>/`.
3. Acknowledge the event only after the CDN confirmed the purge (events
   arrive at least once; purging twice is harmless).

Other domains write a few objects of their own to the media bucket
(subtitles under `subtitles/<video_id>/`, official stamp art under
`stamps/official/`). They get the same edge headers; their removal is
announced by those domains' own events, not by `media.asset.deleted`.
