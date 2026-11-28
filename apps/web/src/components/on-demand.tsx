import { type ComponentType, use, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import i18n from "#/lib/i18n";
import { loadModule } from "#/lib/load-module";

/** What a part shows when its code did not arrive (see `onDemand`). */
export type WhenFailed =
  /** Nothing: it stays off until it loads. For what nobody asked for (a stamp's animation, a ringing call). */
  | "nothing"
  /** A small 「読み込めなかった…」 in its place, with a button that tries again. For what takes room on a page. */
  | "notice"
  /** A toast with a button that tries again. For what opens on a tap (a sheet, a dialog). */
  | "toast";

/** Parts made so far: each failing one has a toast of its own. */
let parts = 0;

/** A component whose code loads the first time it is shown. */
export type OnDemand<P> = ComponentType<P> & {
  /** Starts loading it now; settles once it has loaded or failed to. */
  preload(): Promise<unknown>;
};

/**
 * `React.lazy` for code split off the app's entry, failing where it is shown:
 * a part whose code does not arrive (the connection dropped right then) shows
 * `whenFailed` instead of taking the app to its error screen, and loads again
 * when the device is back online (`loadModule`), when asked to, or the next
 * time it is shown. Until its first load settles, the nearest `<Suspense>`
 * shows its fallback; once loaded, it renders at once everywhere.
 *
 *   const StaticMapPreview = onDemand(
 *     () => import("#/features/map").then((module) => module.StaticMapPreview),
 *     "notice",
 *   );
 */
export function onDemand<P extends object>(
  load: () => Promise<ComponentType<P>>,
  whenFailed: WhenFailed = "nothing",
): OnDemand<P> {
  let loaded: ComponentType<P> | undefined;
  let loading: Promise<ComponentType<P>> | undefined;
  /** The load a newly shown part waits for: its component, or `undefined` if it failed. */
  let waiting: Promise<ComponentType<P> | undefined> | undefined;
  const toastId = `on-demand:${++parts}`;

  // One load at a time, wherever the part is shown.
  const fetchPart = () => {
    loading ??= load().then(
      (component) => {
        loaded = component;
        return component;
      },
      (error: unknown) => {
        loading = undefined;
        throw error;
      },
    );
    return loading;
  };
  const settled = () => {
    waiting ??= fetchPart().catch(() => undefined);
    return waiting;
  };

  function Part(props: P) {
    const [attempt, setAttempt] = useState(() => (loaded ? undefined : settled()));
    const [retried, setRetried] = useState<ComponentType<P>>();
    const Component = retried ?? (attempt ? use(attempt) : loaded);
    const retry = () => setAttempt(settled());

    useEffect(() => {
      if (Component) return;
      // The next part to be shown loads again, but only once this failure is on screen: a part
      // that suspended on the failed load is rendered again by React, and must not start another.
      if (waiting === attempt) waiting = undefined;
      if (whenFailed === "toast") {
        const t = i18n.getFixedT(null, "common");
        toast.error(t("state.notLoaded"), {
          id: toastId,
          action: { label: t("actions.retry"), onClick: retry },
        });
      }
      const stop = loadModule(fetchPart, (component) => setRetried(() => component));
      return () => {
        stop();
        toast.dismiss(toastId);
      };
    }, [Component, attempt]);

    if (Component) return <Component {...props} />;
    return whenFailed === "notice" ? <NotLoaded onRetry={retry} /> : null;
  }

  return Object.assign(Part, { preload: settled });
}

/** In place of a part that did not load. */
function NotLoaded({ onRetry }: { onRetry(): void }) {
  const { t } = useTranslation("common");
  return (
    <div className="grid min-h-24 place-items-center gap-2 rounded-2xl border-2 border-dashed border-line p-3 text-center">
      <p className="text-sm font-bold text-ink-soft">{t("state.notLoaded")}</p>
      <Button size="sm" variant="secondary" onClick={onRetry}>
        {t("actions.retry")}
      </Button>
    </div>
  );
}
