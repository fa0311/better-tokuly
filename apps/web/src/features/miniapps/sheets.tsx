import type { common } from "@tokuly/api";
import type { Card } from "@tokuly/miniapp/protocol";
import { Check, Send } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "#/components/ui/button";
import { Dialog } from "#/components/ui/dialog";
import { FriendPicker } from "#/features/chat";
import { EmojiText } from "#/features/emoji";
import { useArmed } from "#/lib/armed";
import { type dev, localized, NS } from "./api";
import { AppIcon } from "./parts";

/**
 * The once-per-mini-app question: who is asking (name, icon, developer) and
 * every scope the approved manifest asks for, in the catalogue's words.
 * "Allow" works only once the sheet has been in front for a moment
 * (`useArmed`): the mini app chooses when the sheet appears, and must not be
 * able to time it under a tap meant for itself. Closing it is "not now".
 */
export function ConsentSheet({
  open,
  miniApp,
  onAllow,
  onDeny,
}: {
  open: boolean;
  miniApp: dev.MiniApp;
  onAllow(): void;
  onDeny(): void;
}) {
  const { t } = useTranslation(NS);
  return (
    <Dialog
      open={open}
      onOpenChange={(next) => !next && onDeny()}
      title={t("consent.title", { name: miniApp.name })}
      description={t("consent.lead")}
      footer={open && <ConsentButtons onAllow={onAllow} onDeny={onDeny} />}
    >
      <div className="flex flex-col gap-4">
        <div className="flex items-center gap-3">
          <AppIcon icon={miniApp.icon} name={miniApp.name} className="size-12" />
          <div className="min-w-0">
            <p className="truncate font-black">
              <EmojiText text={miniApp.name} />
            </p>
            <p className="truncate text-sm text-ink-soft">
              {t("byline", { handle: miniApp.developer?.handle ?? "" })} · {miniApp.origin}
            </p>
          </div>
        </div>
        <ul className="flex flex-col gap-2" data-testid="mini-app-scopes">
          {miniApp.scopes.map((scope) => (
            <li key={scope.scope} className="flex gap-2.5 rounded-2xl bg-surface-2 px-3.5 py-3">
              <Check className="mt-0.5 size-4 shrink-0 text-brand" strokeWidth={3} />
              <span className="text-sm font-bold leading-relaxed">
                {localized(scope.description)}
              </span>
            </li>
          ))}
        </ul>
        <p className="text-xs leading-relaxed text-ink-faint">{t("consent.revokeHint")}</p>
      </div>
    </Dialog>
  );
}

function ConsentButtons({ onAllow, onDeny }: { onAllow(): void; onDeny(): void }) {
  const { t } = useTranslation(NS);
  const armed = useArmed();
  return (
    <div className="flex w-full flex-col gap-2">
      <Button block disabled={!armed} onClick={() => armed && onAllow()}>
        {t("consent.allow")}
      </Button>
      <Button block variant="secondary" onClick={onDeny}>
        {t("consent.deny")}
      </Button>
    </div>
  );
}

/** The person picks up to `max` friends; only those are handed to the mini app. */
export function FriendsSheet({
  open,
  miniAppName,
  max,
  onPick,
  onCancel,
}: {
  open: boolean;
  miniAppName: string;
  max: number;
  onPick(friends: common.UserSummary[]): void;
  onCancel(): void;
}) {
  const { t } = useTranslation(NS);
  const [selected, setSelected] = useState<common.UserSummary[]>([]);
  useEffect(() => {
    if (open) setSelected([]);
  }, [open]);
  return (
    <Dialog
      open={open}
      onOpenChange={(next) => !next && onCancel()}
      title={t("friends.title")}
      description={t("friends.lead", { name: miniAppName, max })}
      className="sm:h-[min(40rem,calc(100dvh-4rem))]"
      footer={
        <Button disabled={selected.length === 0} onClick={() => onPick(selected)}>
          {t("friends.give", { count: selected.length })}
        </Button>
      }
    >
      <FriendPicker selected={selected} onChange={setSelected} limit={max} />
    </Dialog>
  );
}

/** What the mini app wants to post, as it will look, and where it goes. */
export function ShareSheet({
  open,
  miniApp,
  card,
  onShare,
  onCancel,
}: {
  open: boolean;
  miniApp: dev.MiniApp;
  card: Card | undefined;
  onShare(): Promise<void>;
  onCancel(): void;
}) {
  const { t } = useTranslation(NS);
  const [sending, setSending] = useState(false);
  const share = async () => {
    setSending(true);
    try {
      await onShare();
    } finally {
      setSending(false);
    }
  };
  return (
    <Dialog
      open={open}
      onOpenChange={(next) => !next && onCancel()}
      title={t("share.title")}
      description={t("share.lead", { name: miniApp.name })}
      footer={
        <Button loading={sending} icon={<Send className="size-4" />} onClick={() => void share()}>
          {t("share.send")}
        </Button>
      }
    >
      {card && (
        <div
          className="rounded-[1.25rem] bg-brand px-3.5 py-2.5 text-on-brand"
          data-testid="mini-app-share-preview"
        >
          <p className="font-black break-words">
            <EmojiText text={card.title} />
          </p>
          {card.text && (
            <p className="whitespace-pre-wrap text-sm break-words">
              <EmojiText text={card.text} />
            </p>
          )}
          <p className="mt-2 flex items-center gap-2 rounded-2xl border-2 border-on-brand/30 bg-on-brand/10 p-2">
            <AppIcon icon={miniApp.icon} name={miniApp.name} className="size-7 rounded-lg" />
            <span className="truncate text-sm font-black">
              <EmojiText text={miniApp.name} />
            </span>
          </p>
        </div>
      )}
    </Dialog>
  );
}
