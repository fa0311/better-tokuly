import type { chat } from "@tokuly/api";
import { Users } from "lucide-react";
import { Avatar } from "#/components/ui/avatar";
import { cn } from "#/lib/cn";
import { isGroup, peerOf } from "./model";
import { useOnline } from "./people";

const BOX = {
  sm: "size-8 rounded-[0.6rem]",
  md: "size-11 rounded-[0.85rem]",
  lg: "size-16 rounded-[1.15rem]",
  xl: "size-24 rounded-[1.6rem]",
} as const;

const PIXELS = { sm: 32, md: 44, lg: 64, xl: 96 } as const;

/** Smallest stored rendition of a group icon that still covers the box on a 2x screen. */
function iconUrl(icon: NonNullable<chat.Conversation["icon"]>, pixels: number): string {
  const fitting = icon.variants
    .filter((variant) => variant.size >= pixels * 2)
    .sort((a, b) => a.size - b.size)[0];
  return fitting?.url ?? icon.url;
}

/**
 * The face of a conversation: the other person (with their online dot), a
 * group's icon, or a collage of its first members when it has none.
 */
export function ConversationAvatar({
  conversation,
  myId,
  size = "md",
  className,
}: {
  conversation: chat.Conversation;
  myId: string;
  size?: keyof typeof BOX;
  className?: string;
}) {
  const peer = peerOf(conversation, myId);
  const online = useOnline(peer?.id);
  if (!isGroup(conversation))
    return (
      <Avatar
        user={peer}
        size={size}
        outlined
        online={online && !conversation.readOnly}
        className={className}
      />
    );

  const box = cn(
    "sticker-sm relative grid shrink-0 overflow-hidden bg-surface-2",
    BOX[size],
    className,
  );
  if (conversation.icon?.url) {
    return (
      <span
        className={box}
        style={
          conversation.icon.dominantColor
            ? { background: conversation.icon.dominantColor }
            : undefined
        }
      >
        <img
          src={iconUrl(conversation.icon, PIXELS[size])}
          alt=""
          className="size-full object-cover"
          loading="lazy"
          decoding="async"
          draggable={false}
        />
      </span>
    );
  }
  const faces = conversation.members.slice(0, 4);
  if (faces.length < 2) {
    return (
      <span className={cn(box, "place-items-center bg-sky text-[#17122b]")}>
        <Users className="size-1/2" />
      </span>
    );
  }
  return (
    <span className={cn(box, "grid-cols-2 gap-px bg-ink", faces.length === 2 && "grid-rows-1")}>
      {faces.map((member, index) => (
        // Three faces: the first takes the whole left half.
        <span
          key={member.id}
          className={cn("overflow-hidden", faces.length === 3 && index === 0 && "row-span-2")}
        >
          <Avatar
            user={member}
            size="xl"
            className="!block size-full [&>span]:!size-full [&>span]:!rounded-none [&>span]:!text-[0.7em]"
          />
        </span>
      ))}
    </span>
  );
}
