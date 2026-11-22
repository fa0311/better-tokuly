import { useInfiniteQuery, useQuery } from "@tanstack/react-query";
import { type chat, toMillis } from "@tokuly/api";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Avatar } from "#/components/ui/avatar";
import { Dialog } from "#/components/ui/dialog";
import { LoadMore } from "#/components/ui/load-more";
import { ListRow, Skeleton } from "#/components/ui/page";
import { Tabs } from "#/components/ui/tabs";
import { chatKeys, messageClient } from "./api";
import { displayName } from "./people";
import { dateTime } from "./text";

function RowsSkeleton() {
  return (
    <div className="flex flex-col gap-2">
      {[0, 1, 2].map((index) => (
        <Skeleton key={index} className="h-12" />
      ))}
    </div>
  );
}

/** Who reacted to a message, all together or one emoji at a time. */
export function ReactionsSheet({
  message,
  onClose,
}: {
  message: chat.Message | null;
  onClose(): void;
}) {
  const { t } = useTranslation("chat");
  // The last message stays while the sheet animates out.
  const [shown, setShown] = useState(message);
  if (message && message !== shown) setShown(message);
  const [emoji, setEmoji] = useState("");
  const messageId = shown?.id ?? "";
  const reactions = useInfiniteQuery({
    queryKey: chatKeys.reactions(messageId, emoji),
    queryFn: ({ pageParam }) =>
      messageClient().listReactions({ messageId, emoji, page: { cursor: pageParam, limit: 50 } }),
    initialPageParam: "",
    getNextPageParam: (last) => last.page?.nextCursor || undefined,
    enabled: message !== null,
  });
  if (!shown) return null;
  const total = shown.reactions.reduce((sum, reaction) => sum + reaction.count, 0);
  const rows = reactions.data?.pages.flatMap((page) => page.reactions) ?? [];

  return (
    <Dialog
      open={message !== null}
      onOpenChange={(open) => !open && onClose()}
      title={t("reactions.title")}
      className="h-[70dvh] sm:h-[32rem]"
    >
      <Tabs<string>
        className="mb-3"
        value={emoji}
        onChange={setEmoji}
        items={[
          { value: "", label: t("reactions.all", { count: total }) },
          ...shown.reactions.map((reaction) => ({
            value: reaction.emoji,
            label: `${reaction.emoji} ${reaction.count}`,
          })),
        ]}
      />
      {reactions.isPending ? (
        <RowsSkeleton />
      ) : (
        <div className="-mx-3" data-testid="reaction-list">
          {rows.map((reaction) => (
            <ListRow
              key={`${reaction.user?.id}:${reaction.emoji}`}
              leading={<Avatar user={reaction.user} size="sm" />}
              title={displayName(reaction.user)}
              trailing={<span className="text-2xl">{reaction.emoji}</span>}
            />
          ))}
          <LoadMore query={reactions} />
        </div>
      )}
    </Dialog>
  );
}

/** Which members of a group have read a message, and how many have not yet. */
export function ReceiptsSheet({
  message,
  onClose,
}: {
  message: chat.Message | null;
  onClose(): void;
}) {
  const { t } = useTranslation("chat");
  const [shown, setShown] = useState(message);
  if (message && message !== shown) setShown(message);
  const messageId = shown?.id ?? "";
  const receipts = useQuery({
    queryKey: chatKeys.receipts(messageId),
    queryFn: () => messageClient().listReadReceipts({ messageId }),
    enabled: message !== null,
    staleTime: 0,
  });
  if (!shown) return null;

  return (
    <Dialog
      open={message !== null}
      onOpenChange={(open) => !open && onClose()}
      title={t("receipts.title")}
      description={
        receipts.data
          ? t("receipts.summary", {
              read: receipts.data.receipts.length,
              unread: receipts.data.unreadMemberCount,
            })
          : undefined
      }
      className="h-[70dvh] sm:h-[32rem]"
    >
      {receipts.isPending ? (
        <RowsSkeleton />
      ) : receipts.data && receipts.data.receipts.length > 0 ? (
        <div className="-mx-3" data-testid="receipt-list">
          {receipts.data.receipts.map((receipt) => (
            <ListRow
              key={receipt.user?.id}
              leading={<Avatar user={receipt.user} size="sm" />}
              title={displayName(receipt.user)}
              subtitle={dateTime(toMillis(receipt.readAt))}
            />
          ))}
        </div>
      ) : (
        <p className="py-8 text-center text-sm text-ink-soft">{t("receipts.none")}</p>
      )}
    </Dialog>
  );
}
