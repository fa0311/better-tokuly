import { useEffect } from "react";
import { api } from "#/lib/api";
import { useMe } from "#/stores/session";
import { applyAccent, storedAccent } from "./appearance/accent";
import { isThemeSetting, setAppearance, useAppearance } from "./appearance/appearance-store";
import { useSettings } from "./data";

/** Which account's look this browser last took over from the server. */
const OWNER_KEY = "tokuly.appearanceOwner";

/**
 * Keeps the look of the app in line with the account while someone is signed
 * in:
 * - the first time an account is used in this browser, its theme and
 *   ドパガキ度 are taken over from the server (afterwards the browser's own
 *   copy leads, as `#/lib/appearance` keeps it, and every change is saved to
 *   the account too);
 * - the account's accent colour is applied;
 * - "system" theme follows the device when it switches between light and dark.
 */
export default function SettingsBackground() {
  const userId = useMe()?.user?.id;
  const appearance = useSettings().data?.appearance;
  const theme = useAppearance((state) => state.theme);

  useEffect(() => {
    if (!userId || !appearance || window.localStorage.getItem(OWNER_KEY) === userId) return;
    window.localStorage.setItem(OWNER_KEY, userId);
    setAppearance({ theme: isThemeSetting(appearance.theme) ? appearance.theme : "system", level: appearance.dopamineLevel });
  }, [userId, appearance]);

  // Signing out ends the browser's claim, so the next sign-in starts from the account again.
  useEffect(
    () => () => {
      if (api().session.state.status === "signedOut") window.localStorage.removeItem(OWNER_KEY);
    },
    [],
  );

  // Until the settings arrive, the colour of the last visit avoids a flash of the default.
  const accent = appearance ? appearance.accentColor : storedAccent();
  useEffect(() => applyAccent(accent), [accent]);

  useEffect(() => {
    if (theme !== "system") return;
    const media = window.matchMedia("(prefers-color-scheme: dark)");
    const follow = () => setAppearance({});
    media.addEventListener("change", follow);
    return () => media.removeEventListener("change", follow);
  }, [theme]);

  return null;
}
