import { useOwnLocationReporting } from "./own-location";

/**
 * Runs on every page while someone is signed in: this device keeps sharing
 * its position with friends, not only while the map is on screen.
 */
export default function MapBackground() {
  useOwnLocationReporting();
  return null;
}
