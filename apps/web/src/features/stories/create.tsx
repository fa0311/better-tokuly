import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { media, notes, social } from "@tokuly/api";
import { Check, GripVertical, ImagePlus, Search } from "lucide-react";
import { useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Avatar } from "#/components/ui/avatar";
import { Button } from "#/components/ui/button";
import { Dialog } from "#/components/ui/dialog";
import { Skeleton } from "#/components/ui/page";
import { Sortable } from "#/components/ui/sortable";
import { Tabs } from "#/components/ui/tabs";
import { EmojiText } from "#/features/emoji";
import { charCount, type Upload, UploadTile, useUploads } from "#/features/notes";
import { api } from "#/lib/api";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { useMe } from "#/stores/session";
import { addStory, storyClient } from "./api";

const friendClient = () => api().service(social.FriendService);

const TITLE_MAX = 140;
const CAPTION_MAX = 200;
const MAX_ITEMS = 10;
/** A close-friends story can be shown to at most this many friends. */
const MAX_AUDIENCE = 150;

type Audience = "friends" | "close";

/**
 * Posts a story (記憶): 1–10 photos or videos in the order they will play,
 * each with an optional caption, under an optional title, for every friend
 * or for the friends picked here.
 */
export function CreateStoryDialog({ onClose }: { onClose(): void }) {
  const { t } = useTranslation("stories");
  const me = useMe();
  const queryClient = useQueryClient();
  const picker = useRef<HTMLInputElement>(null);
  const uploads = useUploads({ purpose: () => media.AssetPurpose.STORY, maxItems: MAX_ITEMS });
  const [title, setTitle] = useState("");
  const [captions, setCaptions] = useState<Record<string, string>>({});
  const [audience, setAudience] = useState<Audience>("friends");
  const [chosen, setChosen] = useState<ReadonlySet<string>>(new Set());
  const [idempotencyKey] = useState(() => crypto.randomUUID());

  const post = useMutation({
    mutationFn: () =>
      storyClient().createStory({
        title: title.trim(),
        items: uploads.uploads.map((upload) => ({
          assetId: upload.assetId ?? "",
          caption: (captions[upload.key] ?? "").trim(),
        })),
        audience:
          audience === "close" ? notes.StoryAudience.CLOSE_FRIENDS : notes.StoryAudience.FRIENDS,
        audienceUserIds: audience === "close" ? [...chosen] : [],
        idempotencyKey,
      }),
    onSuccess: (response) => {
      if (response.story && me?.user) addStory(queryClient, response.story, me.user.id);
      toast.success(t("create.posted"));
      onClose();
    },
    onError: (error) => toast.error(errorMessage(error)),
  });

  const titleLength = charCount(title.trim());
  const ready =
    uploads.uploads.length > 0 &&
    !uploads.busy &&
    titleLength <= TITLE_MAX &&
    (audience === "friends" || chosen.size > 0);

  return (
    <Dialog
      open
      onOpenChange={(open) => !open && !post.isPending && onClose()}
      title={t("create.title")}
      description={t("create.lead")}
      className="sm:w-[min(34rem,calc(100vw-2rem))]"
      footer={
        <>
          <Button variant="ghost" onClick={onClose} disabled={post.isPending}>
            {t("create.cancel")}
          </Button>
          <Button
            variant="lime"
            disabled={!ready}
            loading={post.isPending}
            onClick={() => post.mutate()}
          >
            {t("create.post")}
          </Button>
        </>
      }
    >
      <div
        data-testid="story-composer"
        onDragOver={(event) => event.preventDefault()}
        onDrop={(event) => {
          event.preventDefault();
          uploads.add(event.dataTransfer.files);
        }}
      >
        <input
          ref={picker}
          type="file"
          accept="image/*,video/*"
          multiple
          hidden
          data-testid="story-files"
          onChange={(event) => {
            uploads.add(event.target.files ?? []);
            event.target.value = "";
          }}
        />

        {uploads.uploads.length === 0 ? (
          <button
            type="button"
            onClick={() => picker.current?.click()}
            className="flex w-full flex-col items-center gap-2 rounded-card border-2 border-dashed border-line bg-surface-2 px-4 py-10 text-center hover:border-ink-faint"
          >
            <span className="grid size-14 place-items-center rounded-2xl border-2 border-ink bg-sun text-[#17122b]">
              <ImagePlus className="size-7" />
            </span>
            <span className="font-black">{t("create.pick")}</span>
            <span className="text-sm text-ink-soft">
              {t("create.pickHint", { count: MAX_ITEMS })}
            </span>
          </button>
        ) : (
          <>
            <Sortable<Upload>
              items={uploads.uploads}
              getKey={(upload) => upload.key}
              onReorder={uploads.reorder}
              className="flex flex-col gap-2"
            >
              {(upload, { grip }) => (
                <div className="flex items-center gap-2 rounded-2xl bg-surface-2 p-2">
                  <button
                    type="button"
                    aria-label={t("create.move")}
                    className="grid h-16 w-6 shrink-0 place-items-center rounded-lg text-ink-faint hover:text-ink"
                    {...grip}
                  >
                    <GripVertical className="size-5" />
                  </button>
                  <UploadTile
                    upload={upload}
                    onRemove={() => uploads.remove(upload.key)}
                    className="size-16 shrink-0 rounded-xl"
                  />
                  <input
                    value={captions[upload.key] ?? ""}
                    maxLength={CAPTION_MAX}
                    placeholder={t("create.caption")}
                    aria-label={t("create.caption")}
                    onChange={(event) =>
                      setCaptions({ ...captions, [upload.key]: event.target.value })
                    }
                    className="h-11 min-w-0 flex-1 rounded-control border-2 border-line bg-surface px-3 text-[15px] outline-none placeholder:text-ink-faint focus:border-brand"
                  />
                </div>
              )}
            </Sortable>
            {uploads.canAdd && (
              <button
                type="button"
                onClick={() => picker.current?.click()}
                className="mt-2 flex h-11 w-full items-center justify-center gap-2 rounded-2xl border-2 border-dashed border-line text-sm font-black text-ink-soft hover:border-ink-faint hover:text-ink"
              >
                <ImagePlus className="size-4" />
                {t("create.addMore", { count: MAX_ITEMS - uploads.uploads.length })}
              </button>
            )}
          </>
        )}

        <label className="mt-4 block">
          <span className="flex items-baseline justify-between px-1 text-sm font-bold text-ink-soft">
            {t("create.titleLabel")}
            <span
              className={cn(
                "text-xs tabular-nums",
                titleLength > TITLE_MAX ? "text-danger" : "text-ink-faint",
              )}
            >
              {titleLength}/{TITLE_MAX}
            </span>
          </span>
          <input
            value={title}
            placeholder={t("create.titlePlaceholder")}
            onChange={(event) => setTitle(event.target.value)}
            className="mt-1.5 h-12 w-full rounded-control border-2 border-line bg-surface px-4 text-[15px] outline-none placeholder:text-ink-faint focus:border-brand"
          />
        </label>

        <div className="mt-4">
          <p className="mb-1.5 px-1 text-sm font-bold text-ink-soft">{t("create.audience")}</p>
          <Tabs<Audience>
            value={audience}
            onChange={setAudience}
            items={[
              { value: "friends", label: t("create.everyFriend") },
              {
                value: "close",
                label:
                  chosen.size > 0
                    ? t("create.closeCount", { count: chosen.size })
                    : t("create.close"),
              },
            ]}
          />
          {audience === "close" && <FriendPicker chosen={chosen} onChange={setChosen} />}
        </div>
      </div>
    </Dialog>
  );
}

/** Ticks the friends a close-friends story is shown to. */
function FriendPicker({
  chosen,
  onChange,
}: {
  chosen: ReadonlySet<string>;
  onChange(next: ReadonlySet<string>): void;
}) {
  const { t } = useTranslation("stories");
  const [query, setQuery] = useState("");
  const friends = useQuery({
    queryKey: ["social", "friends", "list", query] as const,
    queryFn: () => friendClient().listFriends({ query, page: { limit: 100 } }),
  });
  const toggle = (userId: string) => {
    const next = new Set(chosen);
    if (next.has(userId)) next.delete(userId);
    else if (next.size < MAX_AUDIENCE) next.add(userId);
    else toast.error(t("create.audienceFull", { count: MAX_AUDIENCE }));
    onChange(next);
  };

  return (
    <div className="mt-2 rounded-2xl bg-surface-2 p-2">
      <label className="flex h-10 items-center gap-2 rounded-control border-2 border-line bg-surface px-3 focus-within:border-brand">
        <Search className="size-4 shrink-0 text-ink-faint" />
        <input
          value={query}
          onChange={(event) => setQuery(event.target.value)}
          placeholder={t("create.searchFriends")}
          aria-label={t("create.searchFriends")}
          className="min-w-0 flex-1 bg-transparent text-sm outline-none placeholder:text-ink-faint"
        />
      </label>
      <ul className="mt-1.5 flex max-h-56 flex-col overflow-y-auto">
        {friends.isPending && <Skeleton className="h-11" />}
        {friends.data?.friends.length === 0 && (
          <li className="px-2 py-3 text-sm text-ink-soft">{t("create.noFriends")}</li>
        )}
        {friends.data?.friends.map(
          (friend) =>
            friend.user && (
              <li key={friend.user.id}>
                {/* biome-ignore lint/a11y/useSemanticElements: a row with a face and a name that is ticked as a whole, not a bare checkbox. */}
                <button
                  type="button"
                  role="checkbox"
                  aria-checked={chosen.has(friend.user.id)}
                  onClick={() => toggle(friend.user!.id)}
                  className="flex w-full items-center gap-2.5 rounded-xl px-2 py-1.5 text-left hover:bg-surface-3"
                >
                  <Avatar user={friend.user} size="sm" />
                  <span className="min-w-0 flex-1 truncate font-bold">
                    <EmojiText text={friend.user.displayName} />
                  </span>
                  <span
                    className={cn(
                      "grid size-6 shrink-0 place-items-center rounded-lg border-2",
                      chosen.has(friend.user.id)
                        ? "border-ink bg-lime text-[#17122b]"
                        : "border-line bg-surface",
                    )}
                  >
                    {chosen.has(friend.user.id) && <Check className="size-4" strokeWidth={3.5} />}
                  </span>
                </button>
              </li>
            ),
        )}
      </ul>
    </div>
  );
}
