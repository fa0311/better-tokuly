/**
 * What the notifications area shares with other features.
 *
 * Anything the server hands out as a "path in the app" (a notification's
 * `path`, a realtime event's target) is opened with these, so every feature
 * lands on the same page for the same path:
 *
 *   const open = useOpenTargetPath();
 *   <button onClick={() => open(event.path)}>…</button>
 */
export { openTargetPath, resolveTargetPath, type Target, useOpenTargetPath } from "./target";
