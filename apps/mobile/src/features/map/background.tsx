import { useMapSync } from "./data";
import { useOwnLocationSharing } from "./sharing";

/**
 * Runs while someone is signed in, on every screen: friends' pins stay
 * current, and this device shares its position once the account said yes.
 */
export default function MapBackground() {
  useMapSync();
  useOwnLocationSharing();
  return null;
}
