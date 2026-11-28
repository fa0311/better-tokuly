import { Link } from "@tanstack/react-router";
import type { ReactNode } from "react";
import { AuthLayout } from "#/components/auth-layout";
import { Avatar } from "#/components/ui/avatar";
import { Button } from "#/components/ui/button";
import { LogoMark } from "#/components/ui/logo";
import { EmojiText } from "#/features/emoji";
import { useTranslation } from "#/lib/use-translation";
import { useMe, useSessionStatus } from "#/stores/session";
import type { dev } from "../api";
import { SquareImage } from "../image-field";
import { Notice } from "../parts";
import { ScopeMeanings } from "../scope-picker";
import { useArmed } from "./armed";
import { isFramed } from "./redirect";

const buttonLink =
  "pressable sticker inline-flex h-12 w-full items-center justify-center rounded-full px-6 font-black";

/**
 * The frame of the consent and device pages. They work for visitors who are
 * not signed in yet (sign in, then come back to `returnTo`), and they never
 * render inside another page's frame: a button that hands out access must
 * not be something a hostile page can lay its own picture over.
 */
export function OAuthPage({
  title,
  returnTo,
  children,
}: {
  title: string;
  /** This page's own address, for the way back from signing in. */
  returnTo: string;
  children: ReactNode;
}) {
  const { t } = useTranslation("dev");
  const status = useSessionStatus();
  const me = useMe();

  if (isFramed())
    return (
      <AuthLayout title={t("oauth.framedTitle")}>
        <Notice tone="danger">{t("oauth.framedBody")}</Notice>
      </AuthLayout>
    );
  if (status === "unknown")
    return (
      <div className="grid h-dvh place-items-center bg-bg">
        <LogoMark className="size-20 animate-float" />
      </div>
    );
  if (status === "signedOut")
    return (
      <AuthLayout title={t("oauth.signIn.title")} lead={t("oauth.signIn.lead")}>
        <div className="flex flex-col gap-3">
          <Link
            to="/login"
            search={{ redirect: returnTo }}
            className={`${buttonLink} bg-brand text-on-brand`}
          >
            {t("oauth.signIn.login")}
          </Link>
          <Link
            to="/signup"
            search={{ redirect: returnTo }}
            className={`${buttonLink} bg-surface text-ink`}
          >
            {t("oauth.signIn.signup")}
          </Link>
        </div>
      </AuthLayout>
    );
  return (
    <AuthLayout
      title={title}
      footer={
        me?.user && (
          <span className="inline-flex items-center gap-2">
            <Avatar user={me.user} size="xs" />
            {t("oauth.signedInAs", { handle: me.user.handle })}
          </span>
        )
      }
    >
      {children}
    </AuthLayout>
  );
}

/** Only links that leave for a real website are shown. */
const httpsUrl = (url: string) => (/^https:\/\//i.test(url) ? url : undefined);

/**
 * Who is asking and for what: the application's name and owner, where the
 * browser goes afterwards, and every requested scope in the words of the
 * scope catalogue.
 */
export function ConsentDetails({ consent }: { consent: dev.Consent }) {
  const { t } = useTranslation("dev");
  const app = consent.app;
  const homepage = httpsUrl(app?.homepageUrl ?? "");
  const privacy = httpsUrl(app?.privacyPolicyUrl ?? "");
  return (
    <div className="flex flex-col gap-4">
      <div className="flex items-center gap-3">
        <SquareImage image={app?.icon} name={app?.name ?? ""} className="size-14" />
        <div className="min-w-0">
          <p className="text-lg leading-snug font-black break-words">
            <EmojiText text={app?.name ?? ""} />
          </p>
          <p className="text-sm text-ink-soft">
            {t("oauth.madeBy")}{" "}
            <span className="font-bold text-ink">
              <EmojiText text={app?.owner?.displayName ?? ""} />
            </span>{" "}
            <span className="font-mono text-xs">@{app?.owner?.handle}</span>
          </p>
        </div>
      </div>
      {app?.description && (
        <p className="text-sm leading-relaxed text-ink-soft">
          <EmojiText text={app.description} />
        </p>
      )}
      {(homepage || privacy) && (
        <p className="flex flex-wrap gap-x-4 gap-y-1 text-sm font-bold">
          {homepage && (
            <a
              href={homepage}
              target="_blank"
              rel="noopener noreferrer"
              className="text-brand underline-offset-2 hover:underline"
            >
              {t("oauth.homepage")}
            </a>
          )}
          {privacy && (
            <a
              href={privacy}
              target="_blank"
              rel="noopener noreferrer"
              className="text-brand underline-offset-2 hover:underline"
            >
              {t("oauth.privacy")}
            </a>
          )}
        </p>
      )}

      <div>
        <h2 className="mb-2 text-sm font-black">{t("oauth.wants")}</h2>
        <ScopeMeanings scopes={consent.scopes} />
      </div>

      {consent.alreadyGranted && <Notice>{t("oauth.alreadyGranted")}</Notice>}
    </div>
  );
}

/**
 * Allow and refuse. "Allow" stays disabled until the page has been in
 * front for a moment (see `useArmed`); refusing is never held back.
 */
export function AnswerButtons({
  busy,
  onAnswer,
}: {
  busy: "approve" | "deny" | null;
  onAnswer(choice: "approve" | "deny"): void;
}) {
  const { t } = useTranslation("dev");
  const armed = useArmed();
  return (
    <div className="flex flex-col gap-2.5">
      <Button
        size="lg"
        block
        loading={busy === "approve"}
        disabled={busy !== null || !armed}
        onClick={() => armed && onAnswer("approve")}
      >
        {t("oauth.allow")}
      </Button>
      <Button
        variant="secondary"
        block
        loading={busy === "deny"}
        disabled={busy !== null}
        onClick={() => onAnswer("deny")}
      >
        {t("oauth.deny")}
      </Button>
    </div>
  );
}
