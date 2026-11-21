import { Link } from "@tanstack/react-router";
import type { ReactNode } from "react";
import { Logo } from "#/components/ui/logo";

/** Centered card on a playful backdrop, shared by every signed-out form. */
export function AuthLayout({ title, lead, children, footer }: { title: string; lead?: ReactNode; children: ReactNode; footer?: ReactNode }) {
  return (
    <main className="relative grid min-h-dvh place-items-center overflow-hidden px-4 py-10">
      <Backdrop />
      <div className="relative w-full max-w-md animate-slide-up">
        <Link to="/" className="mb-6 flex justify-center">
          <Logo />
        </Link>
        <section className="sticker-lg rounded-[2rem] bg-surface p-6 sm:p-8">
          <h1 className="text-2xl font-black leading-snug">{title}</h1>
          {lead && <p className="mt-2 text-sm leading-relaxed text-ink-soft">{lead}</p>}
          <div className="mt-6">{children}</div>
        </section>
        {footer && <div className="mt-5 text-center text-sm font-medium text-ink-soft">{footer}</div>}
      </div>
    </main>
  );
}

/** Soft colour blobs; purely decorative. */
export function Backdrop() {
  return (
    <div aria-hidden className="pointer-events-none absolute inset-0 -z-10 overflow-hidden">
      <div className="absolute -top-24 -left-20 size-80 rounded-full bg-brand/25 blur-3xl" />
      <div className="absolute top-1/3 -right-24 size-96 rounded-full bg-lime/30 blur-3xl" />
      <div className="absolute -bottom-28 left-1/4 size-96 rounded-full bg-pink/20 blur-3xl" />
    </div>
  );
}
