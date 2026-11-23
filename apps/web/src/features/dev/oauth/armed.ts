import { useEffect, useState } from "react";

/** How long the page must have been in front before a button that grants access works. */
export const ARM_DELAY_MS = 600;

/**
 * Whether a button that grants access may be pressed yet.
 *
 * Against double-clickjacking: a hostile page opens the consent page in a
 * pop-up and asks for a double click, so the second click lands on "allow"
 * before the person has seen it. The button only works once this page has
 * been visible and focused for `ARM_DELAY_MS`, counted from when the button
 * appears, and the count starts again whenever the page is hidden or loses
 * focus.
 */
export function useArmed(delayMs = ARM_DELAY_MS): boolean {
  const [armed, setArmed] = useState(false);
  useEffect(() => {
    let timer: ReturnType<typeof setTimeout> | undefined;
    const inFront = () => document.visibilityState === "visible" && document.hasFocus();
    const disarm = () => {
      clearTimeout(timer);
      timer = undefined;
      setArmed(false);
    };
    const rearm = () => {
      disarm();
      if (inFront()) timer = setTimeout(() => setArmed(true), delayMs);
    };
    const onVisibility = () => (document.visibilityState === "visible" ? rearm() : disarm());
    rearm();
    window.addEventListener("focus", rearm);
    window.addEventListener("blur", disarm);
    document.addEventListener("visibilitychange", onVisibility);
    return () => {
      clearTimeout(timer);
      window.removeEventListener("focus", rearm);
      window.removeEventListener("blur", disarm);
      document.removeEventListener("visibilitychange", onVisibility);
    };
  }, [delayMs]);
  return armed;
}
