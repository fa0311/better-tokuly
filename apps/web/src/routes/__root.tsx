import type { QueryClient } from "@tanstack/react-query";
import {
  createRootRouteWithContext,
  HeadContent,
  Outlet,
  Scripts,
  useRouter,
} from "@tanstack/react-router";
import { type ReactNode, useEffect, useState } from "react";
import { I18nextProvider } from "react-i18next";
import { Toaster } from "sonner";
import { PWA_LINKS, PWA_META } from "#/features/pwa/head";
import { BOOT_SCRIPT } from "#/lib/appearance";
import i18n, {
  createI18n,
  type Language,
  loadAllStrings,
  normalizeLanguage,
  stopSuspendingForStrings,
} from "#/lib/i18n";
import { getRequestLanguage } from "#/lib/request-language";
import { startSession } from "#/stores/session";
import appCss from "../styles.css?url";

export const Route = createRootRouteWithContext<{ queryClient: QueryClient }>()({
  // Decided once per page load on the server and handed to the client, so
  // both render the same language.
  loader: async () => {
    // The server holds every string before it renders; the browser loads what it shows.
    if (import.meta.env.SSR) await loadAllStrings();
    return { language: await getRequestLanguage() };
  },
  staleTime: Number.POSITIVE_INFINITY,
  head: () => ({
    meta: [
      { charSet: "utf-8" },
      { name: "viewport", content: "width=device-width, initial-scale=1, viewport-fit=cover" },
      { title: "Tokuly — 友達と位置情報共有を楽しもう!" },
      {
        name: "description",
        content:
          "位置情報共有、チャット、ノート、動画、ライブ、ショートがひとつになったアプリ。Your now to your friends.",
      },
      // Theme colour and the tags that make the installed app look like one.
      ...PWA_META,
    ],
    links: [
      { rel: "stylesheet", href: appCss },
      // Icons, the manifest and the launch images.
      ...PWA_LINKS,
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
        <h1 className="mt-4 text-2xl font-black">
          {noWebGl ? "このブラウザでは地図を表示できません" : "うまくいきませんでした"}
        </h1>
        <p className="mt-2 text-sm leading-relaxed text-ink-soft">
          {noWebGl
            ? "WebGL2 に対応したブラウザ、またはハードウェアアクセラレーションを有効にしてお試しください。"
            : "ページを読み込み直すと直ることがあります。"}
        </p>
        <button
          type="button"
          onClick={reset}
          className="pressable sticker mt-6 inline-flex h-12 items-center rounded-full bg-brand px-6 font-black text-on-brand"
        >
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
        <a
          href="/"
          className="pressable sticker mt-6 inline-flex h-12 items-center rounded-full bg-brand px-6 font-black text-on-brand"
        >
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
  const language =
    loaded?.language ??
    normalizeLanguage(typeof document === "undefined" ? undefined : document.documentElement.lang);
  // Server: a fresh instance per request. Browser: the shared instance, which
  // booted in the same language from <html lang>.
  const instance = typeof window === "undefined" ? createI18n(language) : i18n;
  const nonce = useRouter().options.ssr?.nonce;
  return (
    <html lang={language} suppressHydrationWarning>
      <head>
        {/* static boot script, no user input */}
        <script nonce={nonce} dangerouslySetInnerHTML={{ __html: BOOT_SCRIPT }} />
        <HeadContent />
      </head>
      <body>
        <I18nextProvider i18n={instance}>{children}</I18nextProvider>
        <Scripts />
      </body>
    </html>
  );
}

/** Clear of the floating tab bar (and of a composer in its place), plus the home indicator. */
const ABOVE_TAB_BAR = "calc(5.25rem + env(safe-area-inset-bottom))";

/** Whether the phone layout (tab bar, no rail) is showing; `false` until the browser says so. */
function useIsPhone(): boolean {
  const [phone, setPhone] = useState(false);
  useEffect(() => {
    const query = window.matchMedia("(max-width: 767px)");
    const update = () => setPhone(query.matches);
    update();
    query.addEventListener("change", update);
    return () => query.removeEventListener("change", update);
  }, []);
  return phone;
}

function RootComponent() {
  const phone = useIsPhone();
  useEffect(() => {
    stopSuspendingForStrings();
    startSession();
  }, []);
  return (
    <>
      <Outlet />
      {/*
        On phones a toast at the top would lie on the page header (back, title, menu) for
        seconds, so it rises from the bottom instead and stops above the tab bar. Wide screens
        have room next to the header's controls: there it stays at the top.
      */}
      <Toaster
        position={phone ? "bottom-center" : "top-center"}
        offset={phone ? { bottom: ABOVE_TAB_BAR } : undefined}
        mobileOffset={{ bottom: ABOVE_TAB_BAR, left: 12, right: 12 }}
        toastOptions={{
          classNames: {
            toast: "!sticker !rounded-2xl !bg-surface !text-ink !font-sans !font-bold",
          },
        }}
      />
    </>
  );
}
