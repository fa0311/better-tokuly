import { useEffect } from "react";

/**
 * The colour of the browser's bar, and of the installed app's title and
 * status bar, is the page background of the theme in use. The document
 * starts with the device's colour scheme (see `head.ts`); the app's own theme
 * setting can differ from it, so the colour follows `data-theme` from here.
 */
export function useThemeColor(): void {
  useEffect(() => {
    const root = document.documentElement;
    const apply = () => {
      const background = getComputedStyle(root).getPropertyValue("--bg").trim();
      if (!background) return;
      // Whichever of the tags the browser picked, it now says the same.
      for (const meta of document.querySelectorAll<HTMLMetaElement>('meta[name="theme-color" i]')) meta.content = background;
    };
    apply();
    const observer = new MutationObserver(apply);
    observer.observe(root, { attributes: true, attributeFilter: ["data-theme"] });
    return () => observer.disconnect();
  }, []);
}
