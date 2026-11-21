import type { QueryClient } from "@tanstack/react-query";
import { createRootRouteWithContext, HeadContent, Outlet, Scripts } from "@tanstack/react-router";
import { type ReactNode, useEffect } from "react";
import { I18nextProvider } from "react-i18next";
import { Toaster } from "sonner";
import { BOOT_SCRIPT } from "#/lib/appearance";
import i18n, { createI18n, type Language, normalizeLanguage } from "#/lib/i18n";
import { getRequestLanguage } from "#/lib/request-language";
import { startSession } from "#/stores/session";
import appCss from "../styles.css?url";

export const Route = createRootRouteWithContext<{ queryClient: QueryClient }>()({
  // Decided once per page load on the server and handed to the client, so
  // both render the same language.
  loader: async () => ({ language: await getRequestLanguage() }),
  staleTime: Number.POSITIVE_INFINITY,
  head: () => ({
    meta: [
      { charSet: "utf-8" },
      { name: "viewport", content: "width=device-width, initial-scale=1, viewport-fit=cover" },
      { title: "Tokuly — 友達と位置情報共有を楽しもう!" },
      { name: "description", content: "位置情報共有、チャット、ノート、動画、ライブ、ショートがひとつになったアプリ。Your now to your friends." },
      { name: "theme-color", content: "#7c5cff" },
    ],
    links: [
      { rel: "stylesheet", href: appCss },
      { rel: "icon", type: "image/svg+xml", href: "/icon.svg" },
      { rel: "manifest", href: "/manifest.webmanifest" },
    ],
  }),
  shellComponent: RootDocument,
  component: RootComponent,
  errorComponent: RootError,
  notFoundComponent: NotFound,
});

/** Last-resort screen when a route throws: never a blank page. */
function RootError({ error, reset }: { error: unknown; reset(): void }) {
  // The map needs WebGL2; say so plainly instead of a generic failure.
  const noWebGl = error instanceof Error && /WebGL/i.test(error.message);
  return (
    <main className="grid min-h-dvh place-items-center p-6 text-center">
      <div className="max-w-sm">
        <div className="text-6xl">{noWebGl ? "🗺️" : "😵‍💫"}</div>
        <h1 className="mt-4 text-2xl font-black">{noWebGl ? "このブラウザでは地図を表示できません" : "うまくいきませんでした"}</h1>
        <p className="mt-2 text-sm leading-relaxed text-ink-soft">
          {noWebGl ? "WebGL2 に対応したブラウザ、またはハードウェアアクセラレーションを有効にしてお試しください。" : "ページを読み込み直すと直ることがあります。"}
        </p>
        <button type="button" onClick={reset} className="pressable sticker mt-6 inline-flex h-12 items-center rounded-full bg-brand px-6 font-black text-on-brand">
          もう一度ためす
        </button>
      </div>
    </main>
  );
}

function NotFound() {
  return (
    <main className="grid min-h-dvh place-items-center p-6 text-center">
      <div>
        <div className="text-6xl">🧭</div>
        <h1 className="mt-4 text-2xl font-black">ページが見つかりません</h1>
        <a href="/" className="pressable sticker mt-6 inline-flex h-12 items-center rounded-full bg-brand px-6 font-black text-on-brand">
          ホームへ
        </a>
      </div>
    </main>
  );
}

function RootDocument({ children }: { children: ReactNode }) {
  // While this module is hot-reloaded in development the loader data can be
  // missing for a moment; the document already says which language it is in.
  const loaded = Route.useLoaderData() as { language: Language } | undefined;
  const language = loaded?.language ?? normalizeLanguage(typeof document === "undefined" ? undefined : document.documentElement.lang);
  // Server: a fresh instance per request. Browser: the shared instance, which
  // booted in the same language from <html lang>.
  const instance = typeof window === "undefined" ? createI18n(language) : i18n;
  return (
    <html lang={language} suppressHydrationWarning>
      <head>
        {/* biome-ignore lint/security/noDangerouslySetInnerHtml: static boot script, no user input */}
        <script dangerouslySetInnerHTML={{ __html: BOOT_SCRIPT }} />
        <HeadContent />
      </head>
      <body>
        <I18nextProvider i18n={instance}>{children}</I18nextProvider>
        <Scripts />
      </body>
    </html>
  );
}

function RootComponent() {
  useEffect(() => {
    startSession();
  }, []);
  return (
    <>
      <Outlet />
      <Toaster
        position="top-center"
        toastOptions={{
          classNames: {
            toast: "!sticker !rounded-2xl !bg-surface !text-ink !font-sans !font-bold",
          },
        }}
      />
    </>
  );
}
