import { useQuery } from "@tanstack/react-query";
import { Link, useNavigate } from "@tanstack/react-router";
import type { notes } from "@tokuly/api";
import { Globe, Lock, Paperclip, Plus } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "#/components/ui/button";
import { Dialog } from "#/components/ui/dialog";
import { LoadMore } from "#/components/ui/load-more";
import { EmptyState, Page, Skeleton } from "#/components/ui/page";
import { profileClient } from "#/lib/api";
import type { ProfileSectionProps } from "#/seams/profile";
import { useMe } from "#/stores/session";
import { useClips } from "./clips-api";
import { ClipForm } from "./clips-form";

/** Someone's clips (`handle`), or the caller's own with a way to make one. */
export function ClipsScreen({ handle }: { handle?: string }) {
  const { t } = useTranslation("clips");
  const me = useMe();
  const navigate = useNavigate();
  const [creating, setCreating] = useState(false);
  const own = !handle || handle === me?.user?.handle;
  const owner = useQuery({
    queryKey: ["notes", "clips", "owner", handle ?? ""],
    queryFn: async () =>
      (await profileClient().getProfile({ by: { case: "handle", value: handle ?? "" } })).profile
        ?.user ?? null,
    enabled: !own,
  });
  const userId = own ? "" : (owner.data?.id ?? "");
  const clips = useClips(userId, "", own ? Boolean(me?.user) : Boolean(userId));
  const rows = clips.data?.pages.flatMap((page) => page.clips) ?? [];
  const title = own
    ? t("mine")
    : t("userTitle", { name: owner.data?.displayName || owner.data?.handle || handle });

  return (
    <Page
      title={title}
      back="/notes"
      actions={
        own && (
          <Button size="sm" icon={<Plus className="size-4" />} onClick={() => setCreating(true)}>
            {t("create")}
          </Button>
        )
      }
    >
      {own && <p className="mb-3 text-sm font-bold text-ink-soft">{t("lead")}</p>}
      {owner.isError || (owner.isSuccess && !owner.data) ? (
        <EmptyState emoji="🔍" title={t("notFound")} />
      ) : (
        <div className="flex flex-col gap-3">
          {clips.isPending &&
            Array.from({ length: 3 }, (_, index) => (
              <Skeleton key={index} className="h-20 rounded-card" />
            ))}
          {clips.isSuccess && rows.length === 0 && (
            <EmptyState
              emoji="📎"
              title={own ? t("empty") : t("emptyOther")}
              body={own ? t("emptyBody") : undefined}
            />
          )}
          {rows.map((clip) => (
            <ClipRow key={clip.id} clip={clip} />
          ))}
          <LoadMore query={clips} />
        </div>
      )}
      <Dialog open={creating} onOpenChange={setCreating} title={t("form.createTitle")}>
        <ClipForm
          onCancel={() => setCreating(false)}
          onSaved={(clip) => {
            setCreating(false);
            void navigate({ to: "/notes/clips/$clipId", params: { clipId: clip.id } });
          }}
        />
      </Dialog>
    </Page>
  );
}

export function ClipRow({ clip }: { clip: notes.Clip }) {
  const { t } = useTranslation("clips");
  const Audience = clip.public ? Globe : Lock;
  return (
    <Link
      to="/notes/clips/$clipId"
      params={{ clipId: clip.id }}
      className="sticker pressable flex items-center gap-3 rounded-card bg-surface px-4 py-3"
    >
      <span className="grid size-11 shrink-0 place-items-center rounded-2xl border-2 border-ink bg-sky text-[#17122b]">
        <Paperclip className="size-5" strokeWidth={2.5} />
      </span>
      <span className="min-w-0 flex-1">
        <span className="block truncate font-black">{clip.name}</span>
        {clip.description && (
          <span className="block truncate text-sm text-ink-soft">{clip.description}</span>
        )}
        <span className="mt-0.5 flex items-center gap-2 text-xs font-bold text-ink-faint">
          <Audience className="size-3.5" aria-hidden />
          {clip.public ? t("public") : t("private")}
          <span>{t("noteCount", { count: clip.noteCount })}</span>
        </span>
      </span>
    </Link>
  );
}

/** 「クリップ」 among a profile's actions: their public clips (or one's own). */
export function ProfileClipsLink({ user, isSelf }: ProfileSectionProps) {
  const { t } = useTranslation("clips");
  return (
    <Link
      to="/notes/clips"
      search={isSelf ? {} : { user: user.handle }}
      className="pressable sticker inline-flex h-11 items-center gap-2 rounded-full bg-surface px-5 font-bold"
    >
      <Paperclip className="size-4" />
      {t("profileLink")}
    </Link>
  );
}
