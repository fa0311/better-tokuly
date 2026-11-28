import { useSyncExternalStore } from "react";
// biome-ignore lint/style/noRestrictedImports: the one place that wraps it.
import { useTranslation as useStrings } from "react-i18next";

/**
 * react-i18next's `useTranslation`, suspending for a namespace that has not
 * loaded only while its component hydrates.
 *
 * Hydrating, a component must say what the server said, so it waits for its
 * strings. Anything rendered after that shows nothing for the moment a
 * namespace takes and renders again when it arrives (`bindI18n: "loaded"`):
 * suspending then would hide the whole screen under the nearest boundary,
 * and take the focus from whatever was being typed into (a comment box that
 * opened its stamp button on focus lost what was typed next).
 */
export const useTranslation: typeof useStrings = (namespace, options) =>
  useStrings(namespace, { useSuspense: useHydrating(), ...options });

const subscribe = () => () => {};

/** Whether React is hydrating the calling component: it reads the server snapshot only then. */
function useHydrating(): boolean {
  return useSyncExternalStore(
    subscribe,
    () => false,
    () => true,
  );
}
