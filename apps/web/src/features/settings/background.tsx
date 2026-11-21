import { useQueryClient } from "@tanstack/react-query";
import { useEffect } from "react";
import { api } from "#/lib/api";
import { useMe } from "#/stores/session";
import { applyAccent, DEFAULT_ACCENT, storedAccent } from "./appearance/accent";
import { isThemeSetting, setAppearance, useAppearance } from "./appearance/appearance-store";
import { PRIVATE_KEYS, useSettings } from "./data";

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
 * - "system" theme follows the device when it switches between light and dark;
 * - what was read about the account (settings, devices, passkeys, blocks) is
 *   forgotten when it signs out, so the next person on this browser starts clean.
 */
export default function SettingsBackground() {
  const queryClient = useQueryClient();
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
      if (api().session.state.status !== "signedOut") return;
      window.localStorage.removeItem(OWNER_KEY);
      for (const queryKey of PRIVATE_KEYS) queryClient.removeQueries({ queryKey });
      // The signed-out pages wear the default colour again.
      applyAccent(DEFAULT_ACCENT);
    },
    [queryClient],
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
