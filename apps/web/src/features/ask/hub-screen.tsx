import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import type { ask } from "@tokuly/api";
import { Flame, Inbox, MessagesSquare, Send, Settings, Share2, Sparkles, Vote } from "lucide-react";
import { type ComponentType, useState } from "react";
import { useTranslation } from "react-i18next";
import { CountBadge } from "#/components/ui/badge";
import { Button } from "#/components/ui/button";
import { Page, Skeleton } from "#/components/ui/page";
import { SwitchRow } from "#/components/ui/switch";
import { cn } from "#/lib/cn";
import { useMe } from "#/stores/session";
import {
  askKeys,
  boxClient,
  useBox,
  useFlameSummary,
  useFriendPrompts,
  useInboxCounts,
  useSettings,
} from "./api";
import { SectionTitle, toastError } from "./parts";
import { PromptCard } from "./prompt-card";
import { BoxShareSheet } from "./share-sheet";

/** `/ask`: the viewer's own question box and the doors to everything around it. */
export function HubScreen() {
  const { t } = useTranslation("ask");
  const queryClient = useQueryClient();
  const me = useMe();
  const settings = useSettings();
  const counts = useInboxCounts().data;
  const flames = useFlameSummary().data;
  const friendPrompts = useFriendPrompts();
  const myBox = useBox(me?.user?.id);
  const [sharing, setSharing] = useState(false);
  const handle = me?.user?.handle ?? "";
  const open = settings.data?.boxOpen ?? false;

  const setOpen = useMutation({
    mutationFn: async (boxOpen: boolean) =>
      (await boxClient().updateSettings({ boxOpen })).settings!,
    onMutate: (boxOpen) =>
      queryClient.setQueryData<ask.Settings>(askKeys.settings, (known) =>
        known ? { ...known, boxOpen } : known,
      ),
    onSuccess: (saved) => queryClient.setQueryData(askKeys.settings, saved),
    onError: (error) => {
      toastError(error);
      void queryClient.invalidateQueries({ queryKey: askKeys.settings });
    },
  });
  const prompts = friendPrompts.data?.pages.flatMap((page) => page.prompts).slice(0, 3) ?? [];

  return (
    <Page
      title={t("title")}
      wide
      back="/me"
      actions={
        <Link
          to="/ask/settings"
          aria-label={t("hub.settings")}
          className="grid size-9 place-items-center rounded-full hover:bg-surface-2"
        >
          <Settings className="size-5" />
        </Link>
      }
    >
      <div className="grid grid-cols-1 items-start gap-x-8 gap-y-5 lg:grid-cols-[minmax(0,26rem)_minmax(0,1fr)]">
        <section className="sticker rounded-card bg-surface p-4" data-testid="my-box">
          <div className="flex items-center gap-2 px-1">
            <h2 className="min-w-0 flex-1 truncate font-black">{t("hub.myBox")}</h2>
            {settings.data && (
              <span
                className={cn(
                  "rounded-full px-2.5 py-0.5 text-xs font-black",
                  open ? "bg-lime text-[#17122b]" : "bg-surface-3 text-ink-soft",
                )}
                data-testid="box-state"
              >
                {open ? t("hub.open") : t("hub.closed")}
              </span>
            )}
          </div>
          {settings.isPending ? (
            <Skeleton className="mt-3 h-24" />
          ) : (
            <>
              <p
                className={cn(
                  "sticker-sm mt-3 rounded-2xl px-4 py-3 text-lg leading-snug font-black break-words",
                  open ? "bg-sun text-[#17122b]" : "bg-surface-2 text-ink-soft",
                )}
              >
                <span aria-hidden className="mr-1.5">
                  💌
                </span>
                {settings.data?.boxPrompt || t("settings.promptPlaceholder")}
              </p>
              {!open && (
                <p className="mt-3 px-1 text-sm font-bold text-ink-soft">{t("hub.closedLead")}</p>
              )}
              <div className="mt-2">
                <SwitchRow
                  label={t("hub.openSwitch")}
                  hint={t("hub.openHint")}
                  checked={open}
                  onChange={(next) => setOpen.mutate(next)}
                />
              </div>
              <div className="mt-2 flex gap-2">
                <Button
                  className="flex-1"
                  icon={<Share2 className="size-4" />}
                  onClick={() => setSharing(true)}
                >
                  {t("hub.share")}
                </Button>
                <Link
                  to="/ask/settings"
                  className="pressable sticker inline-flex h-11 items-center gap-2 rounded-full bg-surface px-5 font-bold"
                >
                  <Settings className="size-4" />
                  {t("hub.settings")}
                </Link>
              </div>
            </>
          )}
        </section>

        <div>
          <ul className="grid grid-cols-2 gap-3 sm:grid-cols-3">
            <HubTile
              to="/ask/inbox"
              icon={Inbox}
              tint="bg-pink"
              label={t("hub.inbox")}
              lead={
                counts?.unread ? t("hub.inboxNew", { count: counts.unread }) : t("hub.inboxLead")
              }
              badge={counts?.unread}
            />
            <HubTile
              to="/ask/polls"
              icon={Vote}
              tint="bg-lime"
              label={t("hub.polls")}
              lead={t("hub.pollsLead")}
            />
            <HubTile
              to="/ask/flames"
              icon={Flame}
              tint="bg-coral"
              label={t("hub.flames")}
              lead={
                flames?.total ? t("hub.flamesCount", { count: flames.total }) : t("hub.flamesLead")
              }
              badge={flames?.unseen}
            />
            <HubTile
              to="/ask/prompts"
              icon={Sparkles}
              tint="bg-sun"
              label={t("hub.prompts")}
              lead={t("hub.promptsLead")}
            />
            <HubTile
              to="/ask/sent"
              icon={Send}
              tint="bg-sky"
              label={t("hub.sent")}
              lead={t("hub.sentLead")}
            />
            {handle && (
              <HubTile
                to="/ask/u/$handle"
                params={{ handle }}
                icon={MessagesSquare}
                tint="bg-mint"
                label={t("hub.myAnswers")}
                lead={t("hub.myAnswersLead", {
                  count: myBox.data?.answerCount ?? counts?.answered ?? 0,
                })}
              />
            )}
          </ul>

          <SectionTitle
            action={
              <Link to="/ask/prompts" className="text-sm font-bold text-brand">
                {t("profile.more")}
              </Link>
            }
          >
            {t("hub.friendPrompts")}
          </SectionTitle>
          {friendPrompts.isPending ? (
            <Skeleton className="h-20" />
          ) : prompts.length === 0 ? (
            <p className="rounded-card bg-surface px-4 py-6 text-center text-sm font-bold text-ink-soft">
              🍃 {t("hub.friendPromptsEmpty")}
            </p>
          ) : (
            <div className="flex flex-col gap-3">
              {prompts.map((prompt) => (
                <PromptCard key={prompt.id} prompt={prompt} />
              ))}
            </div>
          )}
        </div>
      </div>
      {sharing && handle && (
        <BoxShareSheet
          open
          onOpenChange={setSharing}
          handle={handle}
          prompt={settings.data?.boxPrompt ?? ""}
        />
      )}
    </Page>
  );
}

function HubTile({
  to,
  params,
  icon: Icon,
  tint,
  label,
  lead,
  badge = 0,
}: {
  to: "/ask/inbox" | "/ask/polls" | "/ask/flames" | "/ask/prompts" | "/ask/sent" | "/ask/u/$handle";
  params?: { handle: string };
  icon: ComponentType<{ className?: string }>;
  tint: string;
  label: string;
  lead: string;
  badge?: number;
}) {
  return (
    <li>
      <Link
        to={to}
        params={params}
        className="pressable sticker flex h-32 flex-col justify-between rounded-card bg-surface p-4"
      >
        <span className="flex items-start justify-between">
          <span
            className={cn(
              "grid size-11 place-items-center rounded-2xl border-2 border-ink text-[#17122b]",
              tint,
            )}
          >
            <Icon className="size-6" />
          </span>
          <CountBadge count={badge} />
        </span>
        <span className="min-w-0">
          <span className="block truncate font-black">{label}</span>
          <span className="block truncate text-xs font-bold text-ink-soft">{lead}</span>
        </span>
      </Link>
    </li>
  );
}
