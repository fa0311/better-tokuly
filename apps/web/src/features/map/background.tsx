import { useFriendPositionsHousekeeping } from "./data";
import { useOwnLocationReporting } from "./own-location";

/**
 * Runs on every page while someone is signed in:
 * - this device keeps sharing its position with friends, not only while the
 *   map is on screen — once the signed-in account has said yes here;
 * - friends' positions leave memory the moment they should (someone is no
 *   longer a friend; the account signs out).
 */
export default function MapBackground() {
  useOwnLocationReporting();
  useFriendPositionsHousekeeping();
  return null;
}
