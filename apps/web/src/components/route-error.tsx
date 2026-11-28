import { type ErrorComponentProps, useRouter } from "@tanstack/react-router";

/**
 * What a page shows in its place when it fails: its code did not arrive, or
 * it threw. Every route has one (`defaultErrorComponent`, the root's too: never
 * a blank page), so a failed page leaves the app's layout, the tab bar and
 * the rail in place. "Try again"
 * loads the page again (`router.invalidate()`): resetting the boundary alone
 * would render the same failure.
 */
export function RouteError({ error, reset }: ErrorComponentProps) {
  const router = useRouter();
  // The map needs WebGL2; say so plainly instead of a generic failure.
  const noWebGl = error instanceof Error && /WebGL/i.test(error.message);
  return (
    <div className="grid min-h-dvh place-items-center p-6 text-center">
      <div className="max-w-sm">
        <div className="text-6xl">{noWebGl ? "🗺️" : "😵‍💫"}</div>
        <h1 className="mt-4 text-2xl font-black">
          {noWebGl ? "このブラウザでは地図を表示できません" : "うまくいきませんでした"}
        </h1>
        <p className="mt-2 text-sm leading-relaxed text-ink-soft">
          {noWebGl
            ? "WebGL2 に対応したブラウザ、またはハードウェアアクセラレーションを有効にしてお試しください。"
            : "もう一度ためすと直ることがあるよ。"}
        </p>
        <button
          type="button"
          onClick={() => void router.invalidate().finally(reset)}
          className="pressable sticker mt-6 inline-flex h-12 items-center rounded-full bg-brand px-6 font-black text-on-brand"
        >
          もう一度ためす
        </button>
      </div>
    </div>
  );
}
