import { requireNativeModule } from "expo";

/** The local `tokuly-web-data` module (`modules/tokuly-web-data`). */
interface WebDataModule {
  clearAll(): Promise<void>;
}

let native: WebDataModule | null = null;

/**
 * Removes everything the app's web views stored on the device: cookies (on
 * disk too), localStorage and the other DOM storage, the cache. Never
 * fails: a web view that cannot be cleared is still incognito.
 */
export async function clearWebData(): Promise<void> {
  try {
    native ??= requireNativeModule<WebDataModule>("TokulyWebData");
    await native.clearAll();
  } catch (error) {
    console.warn("Could not clear the web views' data.", error);
  }
}
