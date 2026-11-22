/**
 * What the installable-app area shares with other features.
 *
 * - `useIsStandalone()` / `isStandalone()`: whether this page runs as the
 *   installed app (from the home screen, or in a window of its own), for
 *   anything that should differ there.
 * - `useOnline()`: whether the device has a connection (`false` is certain).
 * - `openInstallGuide()`: opens the step-by-step "add to Home Screen" sheet,
 *   for places that need the app installed first (push on iPhones):
 *
 *     const way = useInstallWay();
 *     {hasGuide(way) && <button onClick={openInstallGuide}>…</button>}
 */
export {
  hasGuide,
  type InstallWay,
  isStandalone,
  openInstallGuide,
  useInstallWay,
  useIsStandalone,
} from "./install";
export { useOnline } from "./online";
