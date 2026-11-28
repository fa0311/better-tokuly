import { useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { common } from "@tokuly/api";
import { cardMessage, miniAppHref, miniAppKeys, nextStep, personOf } from "@tokuly/core/miniapps";
import {
  answer,
  type Call,
  type Card,
  cardTarget,
  type Launch,
  refuse,
  type Scope,
} from "@tokuly/miniapp/protocol";
import { ArrowLeft, Flag, MoreHorizontal, RotateCw, ShieldOff } from "lucide-react";
import { type ReactNode, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { LogoMark } from "#/components/ui/logo";
import { Menu } from "#/components/ui/menu";
import { EmojiText } from "#/features/emoji";
import { ReportDialog } from "#/features/engagement";
import { errorMessage } from "#/lib/errors";
import { useMe, useSessionStatus } from "#/stores/session";
import { type dev, messageClient, miniAppClient, NS, useMiniApp } from "./api";
import { listenToFrame, reply } from "./bridge";
import { AppIcon } from "./parts";
import { ConsentSheet, FriendsSheet, ShareSheet } from "./sheets";

/** What the page reads from its address (`miniAppHref`). */
export interface MiniAppSearch {
  /** A card's path on the mini app's origin. */
  p?: string;
  /** The conversation it was opened from. */
  c?: string;
  /** The approved origin, which the page's policy lets it frame. */
  o?: string;
}

/** A call that waits for the person. */
type Waiting =
  | { id: string; kind: "consent"; scope: Scope; call: Call }
  | { id: string; kind: "friends"; max: number }
  | { id: string; kind: "share"; card: Card };

/**
 * `/miniapps/<id>`: runs an approved mini app in a sandboxed frame, with
 * Tokuly's bar above it, and answers its bridge calls (`docs/miniapps.md`).
 */
export function MiniAppScreen({ appId, search }: { appId: string; search: MiniAppSearch }) {
  const { t } = useTranslation(NS);
  const status = useSessionStatus();
  const query = useMiniApp(status === "signedIn" ? appId : "");

  if (status === "unknown" || (status === "signedIn" && query.isPending))
    return (
      <div className="grid h-dvh place-items-center bg-bg">
        <LogoMark className="size-20 animate-float" />
      </div>
    );
  if (status === "signedOut")
    return (
      <Problem title={t("signIn.title")} body={t("signIn.body")}>
        <Link
          to="/login"
          search={{ redirect: miniAppHref(appId, { path: search.p, conversationId: search.c }) }}
          className="pressable sticker inline-flex h-12 items-center justify-center rounded-full bg-brand px-6 font-black text-on-brand"
        >
          {t("signIn.login")}
        </Link>
      </Problem>
    );
  const miniApp = query.data?.miniApp;
  if (!miniApp)
    return (
      <Problem
        title={t("unavailable.title")}
        body={errorMessage(query.error, { namespace: NS })}
        back={search.c}
      />
    );
  return (
    <Frame
      miniApp={miniApp}
      granted={query.data?.grantedScopes ?? []}
      search={search}
      key={miniApp.launchUrl}
    />
  );
}

function Frame({
  miniApp,
  granted,
  search,
}: {
  miniApp: dev.MiniApp;
  granted: string[];
  search: MiniAppSearch;
}) {
  const { t, i18n } = useTranslation(NS);
  const queryClient = useQueryClient();
  const me = useMe();
  const frame = useRef<HTMLIFrameElement>(null);
  const [waiting, setWaiting] = useState<Waiting | null>(null);
  const [reloads, setReloads] = useState(0);
  const [reporting, setReporting] = useState(false);
  const conversationId = search.c ?? "";
  const back = conversationId ? `/chat/${conversationId}` : "/chat";
  // The address names the origin the page's policy allows; the frame shows
  // only what the API approved. When they differ, the address is corrected.
  const misnamed = search.o !== miniApp.origin;
  useEffect(() => {
    if (misnamed)
      window.location.replace(
        miniAppHref(miniApp.appId, { path: search.p, conversationId, origin: miniApp.origin }),
      );
  }, [misnamed, miniApp.appId, miniApp.origin, search.p, conversationId]);

  const requested = miniApp.scopes.map((scope) => scope.scope);
  const src = (search.p && cardTarget(miniApp.launchUrl, search.p)) || miniApp.launchUrl;
  const launch: Launch = {
    path: new URL(src).pathname + new URL(src).search,
    language: i18n.language.startsWith("ja") ? "ja" : "en",
    inConversation: Boolean(conversationId),
    grantedScopes: granted.filter((scope): scope is Scope => requested.includes(scope)),
  };

  const send = (response: Parameters<typeof reply>[1]) => {
    if (frame.current) reply(frame.current, response);
  };

  /** Runs a call whose scope (if any) is granted. */
  const run = (id: string, call: Call) => {
    switch (call.method) {
      case "hello":
        return send(answer(id, launch));
      case "getProfile":
        return me?.user
          ? send(answer(id, personOf(me.user)))
          : send(refuse(id, "UNAVAILABLE", "not signed in"));
      case "pickFriends":
        return setWaiting({ id, kind: "friends", max: call.max });
      case "shareCard":
        return setWaiting({ id, kind: "share", card: call.card });
      case "close":
        send(answer(id, { closed: true }));
        window.location.assign(back);
    }
  };

  const onCall = (id: string, call: Call) => {
    // One question to the person at a time; the frame can ask again after.
    if (waiting && call.method !== "hello")
      return send(refuse(id, "BUSY", "another call is waiting for the person"));
    const step = nextStep(call, { requested, granted, inConversation: launch.inConversation });
    if (step.kind === "refuse") return send(refuse(id, step.code, step.message));
    if (step.kind === "ask") return setWaiting({ id, kind: "consent", scope: step.scope, call });
    run(id, call);
  };
  const latest = useRef(onCall);
  useEffect(() => {
    latest.current = onCall;
  });
  // A reload is a new frame element to listen to.
  useEffect(() => {
    const element = frame.current;
    if (!element || misnamed || reloads < 0) return;
    return listenToFrame(element, (id, call) => latest.current(id, call));
  }, [misnamed, reloads]);

  const settle = () => setWaiting(null);

  const grant = async () => {
    if (waiting?.kind !== "consent") return;
    const { id, call } = waiting;
    try {
      // OAuth-style: everything the approved manifest asks for, agreed to once.
      const response = await miniAppClient().grantMiniApp({
        appId: miniApp.appId,
        scopes: requested,
      });
      queryClient.setQueryData(miniAppKeys.app(miniApp.appId), {
        miniApp,
        grantedScopes: response.grantedScopes,
      });
      settle();
      run(id, call);
    } catch (error) {
      toast.error(errorMessage(error, { namespace: NS }));
    }
  };

  const shareCard = async () => {
    if (waiting?.kind !== "share") return;
    try {
      await messageClient().sendMessage({
        conversationId,
        clientId: crypto.randomUUID(),
        text: cardMessage(waiting.card, window.location.origin, miniApp.appId),
      });
      send(answer(waiting.id, { shared: true }));
      settle();
      toast.success(t("share.done"));
    } catch (error) {
      toast.error(errorMessage(error, { namespace: NS }));
    }
  };

  const cancel = (code: "CONSENT_DENIED" | "CANCELLED") => {
    if (waiting) send(refuse(waiting.id, code, "the person said no"));
    settle();
  };

  const revoke = async () => {
    try {
      await miniAppClient().revokeMiniAppGrant({ appId: miniApp.appId });
      await queryClient.invalidateQueries({ queryKey: miniAppKeys.app(miniApp.appId) });
      toast.success(t("menu.revoked"));
    } catch (error) {
      toast.error(errorMessage(error, { namespace: NS }));
    }
  };

  return (
    <div className="flex h-dvh flex-col bg-bg sm:items-center sm:justify-center sm:p-6">
      <div className="flex min-h-0 w-full flex-1 flex-col sm:sticker-lg sm:max-h-[52rem] sm:max-w-md sm:overflow-hidden sm:rounded-[2rem]">
        <header className="flex items-center gap-2 border-b-2 border-line bg-surface px-2 py-2">
          <a
            href={back}
            aria-label={t("back")}
            className="grid size-10 shrink-0 place-items-center rounded-full hover:bg-surface-2"
          >
            <ArrowLeft className="size-5" />
          </a>
          <AppIcon icon={miniApp.icon} name={miniApp.name} className="size-8 rounded-lg" />
          <div className="min-w-0 flex-1 leading-tight">
            <p className="truncate font-black">
              <EmojiText text={miniApp.name} />
            </p>
            <p className="truncate text-[11px] font-bold text-ink-faint">
              {t("byline", { handle: miniApp.developer?.handle ?? "" })}
            </p>
          </div>
          <Menu
            trigger={
              <Button variant="ghost" size="sm" aria-label={t("menu.open")}>
                <MoreHorizontal className="size-5" />
              </Button>
            }
            actions={[
              {
                label: t("menu.reload"),
                icon: <RotateCw className="size-4" />,
                onSelect: () => setReloads(reloads + 1),
              },
              {
                label: t("menu.revoke"),
                icon: <ShieldOff className="size-4" />,
                danger: true,
                disabled: launch.grantedScopes.length === 0,
                onSelect: () => void revoke(),
              },
              {
                label: t("menu.report"),
                icon: <Flag className="size-4" />,
                danger: true,
                onSelect: () => setReporting(true),
              },
            ]}
          />
        </header>
        {misnamed ? (
          <div className="grid flex-1 place-items-center">
            <LogoMark className="size-16 animate-float" />
          </div>
        ) : (
          <iframe
            key={reloads}
            ref={frame}
            src={src}
            title={miniApp.name}
            // Scripts and forms only: an opaque origin (no cookies or storage of
            // this site), no top navigation, no pop-ups, no dialogs.
            sandbox="allow-scripts allow-forms"
            allow=""
            // The SDK learns which Tokuly framed it from the referrer's origin.
            referrerPolicy="origin"
            className="min-h-0 w-full flex-1 border-0 bg-white"
            data-testid="mini-app-frame"
          />
        )}
      </div>

      <ReportDialog
        subject={{ type: common.SubjectType.APP, id: miniApp.appId }}
        open={reporting}
        onOpenChange={setReporting}
        label={miniApp.name}
      />
      <ConsentSheet
        open={waiting?.kind === "consent"}
        miniApp={miniApp}
        onAllow={() => void grant()}
        onDeny={() => cancel("CONSENT_DENIED")}
      />
      <FriendsSheet
        open={waiting?.kind === "friends"}
        miniAppName={miniApp.name}
        max={waiting?.kind === "friends" ? waiting.max : 1}
        onPick={(friends: common.UserSummary[]) => {
          if (waiting) send(answer(waiting.id, friends.map(personOf)));
          settle();
        }}
        onCancel={() => cancel("CANCELLED")}
      />
      <ShareSheet
        open={waiting?.kind === "share"}
        miniApp={miniApp}
        card={waiting?.kind === "share" ? waiting.card : undefined}
        onShare={shareCard}
        onCancel={() => cancel("CANCELLED")}
      />
    </div>
  );
}

function Problem({
  title,
  body,
  back,
  children,
}: {
  title: string;
  body: string;
  back?: string;
  children?: ReactNode;
}) {
  const { t } = useTranslation(NS);
  return (
    <div className="grid h-dvh place-items-center bg-bg px-6">
      <div className="flex max-w-sm flex-col items-center gap-3 text-center">
        <LogoMark className="size-16" />
        <h1 className="text-xl font-black">{title}</h1>
        <p className="text-sm leading-relaxed text-ink-soft">{body}</p>
        {children ?? (
          <a
            href={back ? `/chat/${back}` : "/chat"}
            className="pressable sticker inline-flex h-11 items-center rounded-full bg-surface px-5 font-black"
          >
            {t("back")}
          </a>
        )}
      </div>
    </div>
  );
}
