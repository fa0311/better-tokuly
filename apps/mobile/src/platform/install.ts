import { File, Paths } from "expo-file-system";
import { writeSecure } from "./secure-storage";

/** Lives in the app's own storage, which an uninstall deletes (the iOS keychain is kept). */
const MARKER = "tokuly.installed";

/**
 * On the first launch of an install, forgets what a previous install of the
 * app left in secure storage: the iOS keychain outlives an uninstall, and
 * a reinstalled app must not sign anyone in by itself.
 */
export async function forgetPreviousInstall(
  keys: readonly string[],
  /** Runs first, on the first launch only: what must be kept of what is about to go. */
  beforeForgetting: () => Promise<void> = async () => {},
): Promise<void> {
  const marker = new File(Paths.document, MARKER);
  if (marker.exists) return;
  await beforeForgetting();
  await Promise.all(keys.map((key) => writeSecure(key, null)));
  marker.create();
}
