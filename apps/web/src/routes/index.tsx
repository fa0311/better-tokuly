import { createFileRoute, Link, Navigate } from "@tanstack/react-router";
import { Clapperboard, Footprints, MapPinned, MessageCircleHeart } from "lucide-react";
import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { Backdrop } from "#/components/auth-layout";
import { Logo } from "#/components/ui/logo";
import { cn } from "#/lib/cn";
import { useSessionStatus } from "#/stores/session";

export const Route = createFileRoute("/")({ component: Landing });

function Landing() {
  const { t } = useTranslation("auth");
  const status = useSessionStatus();
  if (status === "signedIn") return <Navigate to="/map" replace />;

  return (
    <main className="relative min-h-dvh overflow-hidden">
      <Backdrop />
      <header className="mx-auto flex max-w-6xl items-center justify-between px-5 py-5">
        <Logo />
        <Link to="/login" className="pressable sticker-sm rounded-full bg-surface px-5 py-2 text-sm font-bold">
          {t("landing.signIn")}
        </Link>
      </header>

      <section className="mx-auto grid max-w-6xl items-center gap-10 px-5 pt-6 pb-16 lg:grid-cols-[1.05fr_1fr] lg:pt-14">
        <div className="animate-slide-up">
          <p className="mb-4 inline-flex rotate-[-2deg] rounded-full bg-lime px-4 py-1.5 text-sm font-black text-[#17122b] sticker-sm">
            Your now to your friends.
          </p>
          <h1 className="text-5xl leading-[1.1] font-black tracking-tight sm:text-6xl">{t("landing.headline")}</h1>
          <p className="mt-5 max-w-md text-lg leading-relaxed text-ink-soft">{t("landing.lead")}</p>
          <div className="mt-8 flex flex-wrap gap-3">
            <Link to="/signup" className="pressable sticker inline-flex h-14 items-center rounded-full bg-brand px-8 text-lg font-black text-on-brand">
              {t("landing.start")}
            </Link>
            <Link to="/login" className="pressable sticker inline-flex h-14 items-center rounded-full bg-surface px-8 text-lg font-black">
              {t("landing.signIn")}
            </Link>
          </div>
        </div>
        <MapPreview />
      </section>

      <section className="mx-auto grid max-w-6xl gap-4 px-5 pb-20 sm:grid-cols-2 lg:grid-cols-4">
        <Feature icon={<MapPinned />} tint="bg-sky" title={t("landing.features.map.title")} body={t("landing.features.map.body")} />
        <Feature icon={<MessageCircleHeart />} tint="bg-pink" title={t("landing.features.chat.title")} body={t("landing.features.chat.body")} />
        <Feature icon={<Clapperboard />} tint="bg-sun" title={t("landing.features.watch.title")} body={t("landing.features.watch.body")} />
        <Feature icon={<Footprints />} tint="bg-lime" title={t("landing.features.footprints.title")} body={t("landing.features.footprints.body")} />
      </section>
      <footer className="pb-10 text-center text-sm font-bold text-ink-faint">Made by LOVE ❤</footer>
    </main>
  );
}

function Feature({ icon, tint, title, body }: { icon: ReactNode; tint: string; title: string; body: string }) {
  return (
    <article className="sticker rounded-card bg-surface p-5">
      <div className={cn("sticker-sm mb-4 grid size-12 place-items-center rounded-2xl text-[#17122b]", tint)}>{icon}</div>
      <h2 className="text-lg font-black">{title}</h2>
      <p className="mt-1.5 text-sm leading-relaxed text-ink-soft">{body}</p>
    </article>
  );
}

/** A little illustrated map with friend pins: what the app is, at a glance. */
function MapPreview() {
  const pins = [
    { left: "18%", top: "24%", tint: "bg-pink", emoji: "🎧", delay: "0s", label: "NOW" },
    { left: "62%", top: "16%", tint: "bg-sun", emoji: "🍜", delay: "0.6s", label: "12分" },
    { left: "40%", top: "52%", tint: "bg-sky", emoji: "🚲", delay: "1.2s", label: "18 KM/H" },
    { left: "72%", top: "64%", tint: "bg-lime", emoji: "😴", delay: "1.8s", label: "3時間" },
  ];
  return (
    <div className="sticker-lg relative mx-auto aspect-[4/5] w-full max-w-md overflow-hidden rounded-[2.5rem] bg-[#dff3e4] dark:bg-[#1b2a33]">
      <svg viewBox="0 0 400 500" className="absolute inset-0 size-full" aria-hidden>
        <path d="M-20 140 C80 110 150 190 260 150 S420 120 440 170" fill="none" stroke="#fff" strokeWidth="26" opacity="0.9" />
        <path d="M120 -20 C100 120 190 220 160 340 S110 480 150 540" fill="none" stroke="#fff" strokeWidth="20" opacity="0.9" />
        <path d="M-20 380 C100 350 220 420 420 360" fill="none" stroke="#fff" strokeWidth="16" opacity="0.9" />
        <path d="M250 500 C270 400 330 330 420 300" fill="none" stroke="#9ad7ff" strokeWidth="46" opacity="0.8" />
        <rect x="200" y="210" width="90" height="70" rx="18" fill="#bfe8c8" />
        <rect x="30" y="230" width="70" height="90" rx="18" fill="#bfe8c8" />
      </svg>
      {pins.map((pin) => (
        <div key={pin.left} className="absolute animate-float" style={{ left: pin.left, top: pin.top, animationDelay: pin.delay }}>
          <div className={cn("sticker grid size-16 place-items-center rounded-[1.2rem] text-3xl", pin.tint)}>{pin.emoji}</div>
          <div className="mx-auto -mt-1 h-3 w-0.5 bg-ink" />
          <div className="mx-auto w-max rounded-full bg-ink px-2.5 py-0.5 text-[11px] font-black text-surface">{pin.label}</div>
        </div>
      ))}
      <div className="glass sticker-sm absolute inset-x-4 bottom-4 flex items-center gap-3 rounded-3xl p-3">
        <div className="sticker-sm grid size-11 place-items-center rounded-2xl bg-brand text-xl">⚡</div>
        <div className="text-sm font-black leading-tight">
          3人が集合中!
          <div className="text-xs font-bold text-ink-soft">渋谷区 · たった今</div>
        </div>
      </div>
    </div>
  );
}
