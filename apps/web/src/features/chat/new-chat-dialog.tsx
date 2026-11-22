import { useQuery, useQueryClient } from "@tanstack/react-query";
import type { common } from "@tokuly/api";
import { ChevronLeft, ChevronRight, UsersRound } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Avatar } from "#/components/ui/avatar";
import { Button } from "#/components/ui/button";
import { Dialog } from "#/components/ui/dialog";
import { TextField } from "#/components/ui/field";
import { EmptyState, Skeleton } from "#/components/ui/page";
import { EmojiText } from "#/features/emoji";
import { profileClient } from "#/lib/api";
import { useMe } from "#/stores/session";
import { conversationClient, useCapabilities } from "./api";
import { putConversation } from "./conversations";
import { errorMessage } from "./errors";
import { FriendPicker } from "./friend-picker";
import { GroupIconPicker } from "./group-icon";
import { useOpenDirect } from "./open-direct";
import { SearchField, SectionTitle } from "./parts";
import { displayName, useFriends } from "./people";

/**
 * "New chat": pick a friend (or find anyone by handle) to open the direct
 * conversation with them, or switch to making a group.
 */
export function NewChatDialog({
  open,
  onOpenChange,
  onOpen,
}: {
  open: boolean;
  onOpenChange(open: boolean): void;
  onOpen(conversationId: string): void;
}) {
  const { t } = useTranslation("chat");
  const [mode, setMode] = useState<"direct" | "group">("direct");
  // Each opening starts from the friend list again.
  useEffect(() => {
    if (open) setMode("direct");
  }, [open]);
  const done = (conversationId: string) => {
    onOpenChange(false);
    onOpen(conversationId);
  };

  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={
        mode === "group" ? (
          <span className="flex items-center gap-1">
            <button
              type="button"
              aria-label={t("actions.back")}
              onClick={() => setMode("direct")}
              className="-ml-2 grid size-9 place-items-center rounded-full hover:bg-surface-2"
            >
              <ChevronLeft className="size-6" />
            </button>
            {t("group.createTitle")}
          </span>
        ) : (
          t("new.title")
        )
      }
      className="sm:h-[min(42rem,calc(100dvh-4rem))]"
    >
      {mode === "direct" ? (
        <StartDirect onGroup={() => setMode("group")} onDone={done} />
      ) : (
        <CreateGroup onDone={done} />
      )}
    </Dialog>
  );
}

function PersonRow({
  user,
  subtitle,
  disabled,
  onClick,
}: {
  user: common.UserSummary;
  subtitle?: string;
  disabled?: boolean;
  onClick(): void;
}) {
  return (
    <button
      type="button"
      disabled={disabled}
      onClick={onClick}
      className="flex w-full items-center gap-3 rounded-2xl px-3 py-2 text-left transition-colors hover:bg-surface-2 disabled:opacity-60"
    >
      <Avatar user={user} outlined />
      <span className="min-w-0 flex-1">
        <span className="block truncate font-bold">
          <EmojiText text={displayName(user)} />
        </span>
        <span className="block truncate text-sm text-ink-soft">
          {subtitle ?? `@${user.handle}`}
        </span>
      </span>
    </button>
  );
}

function StartDirect({
  onGroup,
  onDone,
}: {
  onGroup(): void;
  onDone(conversationId: string): void;
}) {
  const { t } = useTranslation("chat");
  const me = useMe();
  const [query, setQuery] = useState("");
  const [debounced, setDebounced] = useState("");
  const [starting, setStarting] = useState(false);
  const openDirect = useOpenDirect();
  useEffect(() => {
    const timer = setTimeout(() => setDebounced(query.trim().replace(/^@/, "")), 250);
    return () => clearTimeout(timer);
  }, [query]);

  const friends = useFriends(debounced);
  const friendUsers = (friends.data?.friends ?? []).flatMap((friend) =>
    friend.user ? [friend.user] : [],
  );
  // Anyone can be found by handle; whether they can be written to is up to their privacy settings.
  const others = useQuery({
    queryKey: ["identity", "search", debounced],
    queryFn: () => profileClient().searchUsers({ query: debounced }),
    enabled: debounced.length >= 2,
  });
  const strangers = (others.data?.results ?? []).flatMap((result) =>
    result.user &&
    result.user.id !== me?.user?.id &&
    !friendUsers.some((friend) => friend.id === result.user!.id)
      ? [result.user]
      : [],
  );

  const start = async (user: common.UserSummary) => {
    setStarting(true);
    const conversation = await openDirect(user.id);
    setStarting(false);
    if (conversation) onDone(conversation.id);
  };

  return (
    <div className="flex flex-col gap-3">
      <SearchField value={query} onChange={setQuery} placeholder={t("new.search")} />
      {!debounced && (
        <button
          type="button"
          onClick={onGroup}
          className="pressable sticker flex items-center gap-3 rounded-2xl bg-surface px-3 py-2.5 text-left"
        >
          <span className="sticker-sm grid size-11 shrink-0 place-items-center rounded-[0.85rem] bg-lime text-[#17122b]">
            <UsersRound className="size-5" />
          </span>
          <span className="min-w-0 flex-1">
            <span className="block font-black">{t("group.createTitle")}</span>
            <span className="block truncate text-sm text-ink-soft">{t("group.createLead")}</span>
          </span>
          <ChevronRight className="size-5 shrink-0 text-ink-faint" />
        </button>
      )}
      {friends.isPending ? (
        <div className="flex flex-col gap-2">
          {[0, 1, 2].map((index) => (
            <Skeleton key={index} className="h-14" />
          ))}
        </div>
      ) : (
        <>
          {friendUsers.length > 0 && (
            <div className="-mx-2">
              <SectionTitle>{t("new.friends")}</SectionTitle>
              {friendUsers.map((user) => (
                <PersonRow
                  key={user.id}
                  user={user}
                  disabled={starting}
                  onClick={() => void start(user)}
                />
              ))}
            </div>
          )}
          {strangers.length > 0 && (
            <div className="-mx-2">
              <SectionTitle>{t("new.others")}</SectionTitle>
              {strangers.map((user) => (
                <PersonRow
                  key={user.id}
                  user={user}
                  disabled={starting}
                  onClick={() => void start(user)}
                />
              ))}
            </div>
          )}
          {friendUsers.length === 0 && strangers.length === 0 && !others.isFetching && (
            <EmptyState
              emoji={debounced ? "🔍" : "🫶"}
              title={debounced ? t("new.noMatch") : t("new.noFriends")}
              body={debounced ? t("new.noMatchBody") : t("new.noFriendsBody")}
            />
          )}
        </>
      )}
    </div>
  );
}

function CreateGroup({ onDone }: { onDone(conversationId: string): void }) {
  const { t } = useTranslation("chat");
  const queryClient = useQueryClient();
  const capabilities = useCapabilities();
  const [name, setName] = useState("");
  const [icon, setIcon] = useState<{ assetId: string; url: string } | null>(null);
  const [members, setMembers] = useState<common.UserSummary[]>([]);
  const [creating, setCreating] = useState(false);
  const trimmed = name.trim();

  const create = async () => {
    setCreating(true);
    try {
      const { conversation } = await conversationClient().createGroup({
        name: trimmed,
        memberIds: members.map((member) => member.id),
        iconAssetId: icon?.assetId ?? "",
      });
      if (!conversation) return;
      putConversation(queryClient, conversation);
      toast.success(t("group.created"));
      onDone(conversation.id);
    } catch (error) {
      toast.error(errorMessage(error));
    } finally {
      setCreating(false);
    }
  };

  return (
    <form
      className="flex flex-col gap-4"
      onSubmit={(event) => {
        event.preventDefault();
        if (trimmed && members.length > 0) void create();
      }}
    >
      <div className="flex items-center gap-4">
        <GroupIconPicker
          imageUrl={icon?.url}
          onUploaded={(assetId, url) => setIcon({ assetId, url })}
        />
        <TextField
          className="min-w-0 flex-1"
          label={t("group.name")}
          placeholder={t("group.namePlaceholder")}
          value={name}
          maxLength={50}
          onChange={(event) => setName(event.target.value)}
        />
      </div>
      <div>
        <p className="px-1 pb-2 text-sm font-bold text-ink-soft">
          {t("group.members", { count: members.length })}
        </p>
        <FriendPicker
          selected={members}
          onChange={setMembers}
          limit={capabilities.maxGroupMembers - 1}
        />
      </div>
      <div className="sticky bottom-0 -mx-6 -mb-5 bg-surface px-6 pt-2 pb-5">
        <Button type="submit" block loading={creating} disabled={!trimmed || members.length === 0}>
          {t("group.create")}
        </Button>
      </div>
    </form>
  );
}
