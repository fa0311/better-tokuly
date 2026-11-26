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

Checked before anything is transcoded, on the longest of what the container
header claims, the packets' timestamps (read without decoding; for MP4 both
as the edit list plays them and with the edit list ignored) and, for voice
messages, the decoded length: a header that understates the length, an edit
list that hides packets, or one that repeats a short span for hours (a 2 s
file playing for two hours) does not get a recording through or past the
budget. Measuring stops one second past the limit. A
longer upload fails with `MEDIA_TOO_LONG` (1 s of slack for recorders that
stop a few frames late). Every encoder also reads at most the limit plus
that slack (`-t` on the input), and a video whose renditions reach that cap
is refused with `MEDIA_TOO_LONG` instead of being kept cut down. The daily
transcoding budget is charged with the measured length. The limit is in `CreateUploadResponse.max_duration_seconds`
and `Asset.max_duration_seconds`, so clients can say what it is.

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
variant above what it is shown at (`recipe::largest_image_edge`), and
animations are kept only where they are wanted (`recipe::keeps_animation`):

| Purpose | Largest variant | Animation | Size limit |
|---|---|---|---|
| `COMMENT_IMAGE` (a picture in a comment) | 1024 px | kept | `TOKULY_MEDIA_MAX_COMMENT_IMAGE_MB` |
| `CUSTOM_EMOJI` | 256 px (animated rendition: 256 px too) | kept | `TOKULY_MEDIA_MAX_EMOJI_MB` |
| `COMMUNITY_ICON` | 512 px | kept | `TOKULY_MEDIA_MAX_AVATAR_MB` |
| `COMMUNITY_BANNER` | 2048 px | first frame | `TOKULY_MEDIA_MAX_AVATAR_MB` |
| every other picture | 2048 px | kept for `AVATAR`, `STAMP`, `CHAT_IMAGE`, `NOTE_IMAGE`, `STORY` (animated rendition: 512 px) | by purpose (Configuration) |

These four were added after their domains existed. Before, comments took
`NOTE_IMAGE` / `CHAT_IMAGE`, emoji `STAMP`, community icons `AVATAR` /
`CHANNEL_ICON` and banners `BANNER` / `CHANNEL_BANNER`; the domains still
accept uploads made with those (assets from before, and clients built
before), so nothing stored needs to change.

### The fair transcoding queue

A `media.transcode` job is a turn on the transcoder, not a ticket for its
own asset: the worker that takes it transcodes the waiting video that is
fairest to go next (`lifecycle::claim_next_transcode`). Owners take turns:
each owner's oldest waiting video, ranked by how many of the owner's videos
are running or ahead of it, then by how long the owner has waited (since
the video arrived, or since their last turn when that came later: an owner
just served goes after one who was waiting, and accounts arriving later
never jump one that has waited since before). An owner
with `TOKULY_MEDIA_TRANSCODES_PER_ACCOUNT` (2) transcodes running gets no
more until one ends, and the turn that ends queues another one while videos
are waiting. So one account's backlog never holds up anyone else's first
video. Claims are serialized on an advisory lock. `Media::transcode_next`
takes one turn in place (tests, tooling); `process_now` still processes the
named asset.

On top of the daily byte budget, an account may have
`TOKULY_MEDIA_DAILY_TRANSCODE_MINUTES` (720) minutes of video transcoded in
any 24 hours. A video is booked when probed (`media_transcode_usage`, once
per asset, kept when it fails or is deleted later); one that does not fit
fails with `DAILY_TRANSCODE_LIMIT_EXCEEDED`. Media the server makes itself
(live archives and clips) is not booked.

Timers (worker processes): the janitor sweeps every 5 minutes — aborts
uploads unfinished after `TOKULY_MEDIA_UPLOAD_TTL_HOURS`, removes unheld
FAILED assets after 7 days, deletes READY assets that nothing has held for
`TOKULY_MEDIA_UNATTACHED_TTL_HOURS`, purges objects of deleted assets,
re-queues PROCESSING assets whose worker died, drops transcoding-budget rows older than two days and turn records older than one, drops link preview cache rows
older than 30 days and cached preview pictures older than a day.

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

## Pictures of link previews

A preview's picture and site icon (`LinkPreview.image_url`,
`favicon_url`) are never the other site's addresses: every preview handed
out (`GetLinkPreview`, `tokuly_media::link_preview`, so also what chat and
notes store) points them at this instance,

```
{TOKULY_PUBLIC_URL}{TOKULY_API_PATH_PREFIX}/media/link-image/{image|icon}/{signature}/{base64url of the URL}
```

and the API fetches them on the viewer's behalf (`Media::http_routes`,
`link_preview::image_proxy`). Viewing a preview therefore tells the linked
site nothing about who looked. The route:

- answers only URLs the server signed (an HMAC of kind and URL under
  `TOKULY_SECRET`): it is not an open proxy; anything else is a 404;
- fetches through the egress guard of `tokuly_core::egress` (no private or
  internal addresses, every redirect hop checked), 5 s at most, 8 MB at most;
- bounds what a miss costs: 4 downloads at once per API process (taken
  before anything is downloaded; a miss that finds them all busy is answered
  `503`, `Cache-Control: no-store`, `Retry-After: 60`, and nothing is
  remembered), 2 decodes of those at once, 60 picture fetches a minute per
  site (a budget of its own, so viewers' misses do not starve page
  previews), and 120 misses a minute per client address (per /64 for IPv6;
  cache hits are free);
- serves only raster images (JPEG, PNG, WebP, GIF's first frame, ICO; never
  SVG), decoded within 16 megapixels and re-encoded as WebP at most 1200 px
  (pictures) or 128 px (icons) on the longest edge, so nothing of the file
  itself reaches the viewer;
- keeps the result for 24 hours ("nothing to show" for 30 minutes): the WebP
  in the private bucket as `link-images/<image|icon>/<sha256 of the URL>.webp`,
  a row in `link_preview_images`; the janitor removes expired rows with their
  objects. Responses carry `Cache-Control: public, max-age=86400`.

A picture on this instance's own media origin (`TOKULY_MEDIA_URL`) is never
proxied: the preview keeps its URL, so the picture ends the moment the file
is deleted or taken down (a proxied copy would outlive it by a day). Any
other URL of this instance is dropped, and the route refuses both even when
signed.

Clients show a preview picture only when it is a proxy URL or one of this
instance's own `/media/` files (the web app's `linkPreviewImage`; a separate
media origin is not recognised there, so such pictures are not shown);
previews stored before the proxy existed show without one.

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

Images, animations, video and audio are decoded and re-encoded, so nothing
of the uploaded file survives but the picture and sound: EXIF, GPS
positions, device names and container metadata are gone. Files
(`ASSET_KIND_FILE`: chat files, subtitles, dotLottie stamps) are the
opposite: stored and served byte for byte, **metadata included**. A photo
sent as a chat file keeps its location. Clients therefore send photos as
images, and convert the formats the image pipeline does not decode (HEIC,
HEIF, AVIF, BMP: only JPEG, PNG, WebP and GIF are) to JPEG before upload
instead of falling back to a file; the web app re-encodes them through a
canvas, which drops the metadata too.

## Object layout

Private bucket (presigned access only): `originals/{asset_id}/source[.ext]`
is what the upload URLs write; a completed single-PUT upload is copied to
`originals/{asset_id}/stored-{n}[.ext]` before its bytes are checked, so the
upload URL (valid for an hour) cannot replace a finished file. What a late
PUT leaves under the upload key is removed by the janitor once the URL has
expired.

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

Keys in the media bucket never change their content, but any of them can
be **removed**: the owner deletes the asset, a moderator takes it down, the
janitor collects it. A removed object must stop being served at once, so
nothing may be cached as immutable:

- `assets::delete` commits the deletion, then purges the objects straight
  away, public renditions first (the janitor retries within five minutes if
  the object store fails). From then on the bucket answers `404`.
- The edge (`infra/caddy/Caddyfile`) serves `/media/*` with
  `Cache-Control: public, max-age=300, s-maxage=0, must-revalidate`,
  replacing what an object was stored with (objects are stored with the same
  value, `store::PUBLIC_CACHE`; a test keeps the two equal). Shared caches
  must ask the edge on every request; for an unchanged object that is a
  conditional request answered `304` without a body. Browsers keep their
  copy for at most five minutes. So with the stack as shipped, a deleted
  object is gone for everyone who has not already loaded it, at once.
- The edge also tags every object of an asset with
  `Cache-Tag: asset-<asset_id>` and `Surrogate-Key: asset-<asset_id>`.

### A CDN in front of a self-hosted instance

A CDN that honours the headers above needs nothing else (it revalidates
every request, which still saves the transfer). A CDN configured to cache
`/media/*` for longer than that — an "edge TTL" override, "cache
everything", ignoring `s-maxage` — keeps serving removed media, which turns
every takedown into a no-op. Such a deployment **must purge on deletion**:

1. Consume `media.asset.deleted` from the `EVENTS` JetStream stream (NATS
   subject `evt.media.asset.deleted`, prefixed with `TOKULY_NATS_PREFIX.`
   when that is set) with a durable consumer of its own, so no deletion is
   missed while the hook is down. The payload's `asset_id` is all it needs.
2. Purge everything of that asset: by tag `asset-<asset_id>` where the CDN
   supports tags, otherwise by prefix (or every URL) under
   `{TOKULY_MEDIA_URL}/img/<asset_id>/`, `/anim/<asset_id>/`,
   `/video/<asset_id>/`, `/audio/<asset_id>/` and `/file/<asset_id>/`.
3. Acknowledge the event only after the CDN confirmed the purge; events
   arrive at least once, and purging twice is harmless.

The same holds when `TOKULY_MEDIA_URL` points at a CDN that pulls straight
from the bucket instead of through the edge: that CDN must apply the
`Cache-Control` above (objects stored before this change carry the old
one-year `immutable` value) or purge as described.

Other domains write a few objects of their own to the media bucket
(subtitles under `subtitles/<video_id>/`, official stamp art under
`stamps/official/`). They get the same edge headers; their removal is
announced by those domains' own events, not by `media.asset.deleted`.
