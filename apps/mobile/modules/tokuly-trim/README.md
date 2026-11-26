# tokuly-trim

A local Expo module (autolinked from `apps/mobile/modules`) that cuts a video
file down to a time range, for stories and shorts longer than the server
takes. The JS side is `src/platform/trim.ts`; the trim screen is
`src/features/media/trim-screen.tsx`.

- Android: Media3 Transformer clipping (`androidx.media3:media3-transformer`,
  Apache-2.0), the same Media3 version as expo-video.
- iOS: `AVAssetExportSession` with a `timeRange`; passthrough (no re-encode)
  when the asset allows it.

The output is an MP4 in the app's cache directory.
