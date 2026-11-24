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
| `media.asset.failed` | the upload was rejected or could not be processed | `error` holds the reason (`UNSUPPORTED_CONTENT`, `CORRUPT_MEDIA`, `NOT_A_VIDEO`, `NO_AUDIO`, `IMAGE_TOO_LARGE`, `ANIMATION_TOO_LARGE`, `VIDEO_TOO_LARGE`, `MEDIA_TOO_LONG`, `SIZE_MISMATCH`, `CHECKSUM_MISMATCH`, `PROCESSING_FAILED`) |
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
| `media.transcode` | `ProcessJob { asset_id }` | `TOKULY_MEDIA_VIDEO_CONCURRENCY` | video |

A job claims its asset (`media_assets.heartbeat_at`), so duplicates are
no-ops. Infrastructure failures are retried up to 3 attempts per asset, then
the asset fails with `PROCESSING_FAILED`; a rejected input fails at once.

Timers (worker processes): the janitor sweeps every 5 minutes — aborts
uploads unfinished after `TOKULY_MEDIA_UPLOAD_TTL_HOURS`, removes unheld
FAILED assets after 7 days, deletes READY assets that nothing has held for
`TOKULY_MEDIA_UNATTACHED_TTL_HOURS`, purges objects of deleted assets,
re-queues PROCESSING assets whose worker died, drops link preview cache rows
older than 30 days.

## Link previews of this instance

A link to one of this instance's own pages is never fetched: its preview is
`embed_provider = "tokuly"` and an `embed_id` of `<kind>/<id>` (see
`LinkPreview` in the proto; `/g/<handle>` is `community/<handle>`, a thread
under it `community_thread/<id>`, an event `community_event/<id>`). Nothing
about the page itself is resolved here, because a preview is stored once
and shown to everyone in the conversation: the client asks the owning
domain with the viewer's own permissions, so a hidden community, a private
note or a friends-only video previews as a plain link to whoever may not
read it.

## Configuration (`MediaConfig`)

| Variable | Default | Meaning |
|---|---|---|
| `TOKULY_MEDIA_MAX_AVATAR_MB` | 10 | avatars, banners, channel art, thumbnails, stamps |
| `TOKULY_MEDIA_MAX_IMAGE_MB` | 25 | photos in chats, notes, stories, places, day memos |
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
| `TOKULY_MEDIA_IMAGE_CONCURRENCY` | 4 | parallel `media.process` jobs per worker process |
| `TOKULY_MEDIA_VIDEO_CONCURRENCY` | 1 | parallel `media.transcode` jobs per worker process |
| `TOKULY_MEDIA_FFMPEG_THREADS` | 0 | threads per ffmpeg process (0 = ffmpeg decides) |
| `TOKULY_MEDIA_X264_PRESET` | `veryfast` | x264 preset of the HLS ladder |
| `TOKULY_MEDIA_AV1` | `false` | add an AV1 (SVT-AV1) rendition of the top rung |
| `TOKULY_MEDIA_MAX_VIDEO_HOURS` | 12 | longest video/audio accepted |
| `TOKULY_MEDIA_FFMPEG_PATH` / `TOKULY_MEDIA_FFPROBE_PATH` | `ffmpeg` / `ffprobe` | binaries |
| `TOKULY_MEDIA_WORK_DIR` | system temp | scratch space for transcodes |

## Object layout

Private bucket (presigned access only): `originals/{asset_id}/source[.ext]`
is what the upload URLs write; a completed single-PUT upload is copied to
`originals/{asset_id}/stored-{n}[.ext]` before its bytes are checked, so the
upload URL (valid for an hour) cannot replace a finished file. What a late
PUT leaves under the upload key is removed by the janitor once the URL has
expired.

Media bucket (public, `Cache-Control: public, max-age=31536000, immutable`):

```
img/{asset_id}/{size}-{hash}.webp          image variants (2048/1024/512/256/96 px longest edge, never upscaled)
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
