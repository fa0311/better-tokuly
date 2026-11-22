import { Outlet, useNavigate, useRouterState } from "@tanstack/react-router";
import { useEffect } from "react";
import { cn } from "#/lib/cn";
import { ConversationList } from "./conversation-list";
import { useOpenDirect } from "./open-direct";

/**
 * The chat area: the conversation list next to the open conversation on wide
 * screens, one of the two at a time on phones. `openUser` (the `?user=`
 * search parameter: a handle or a user ID) opens the direct conversation
 * with that person.
 */
export function ChatLayout({ openUser }: { openUser?: string }) {
  const navigate = useNavigate();
  const openDirect = useOpenDirect();
  const pathname = useRouterState({ select: (state) => state.location.pathname });
  const [, , section] = pathname.replace(/\/+$/, "").split("/");
  const inner = Boolean(section);
  const activeId = section && section !== "calls" ? section : undefined;

  const open = (conversationId: string) =>
    void navigate({ to: "/chat/$conversationId", params: { conversationId }, search: {} });

  useEffect(() => {
    if (!openUser) return;
    let cancelled = false;
    void openDirect(openUser).then((conversation) => {
      if (cancelled) return;
      if (conversation)
        void navigate({
          to: "/chat/$conversationId",
          params: { conversationId: conversation.id },
          search: {},
          replace: true,
        });
      else void navigate({ to: "/chat", search: {}, replace: true });
    });
    return () => {
      cancelled = true;
    };
    // `openDirect` and `navigate` are stable in what they do; only a new person starts this again.
  }, [openUser]);

  return (
    <div className="flex h-full">
      <aside
        className={cn(
          "h-full w-full shrink-0 border-line md:w-[21rem] md:border-r-2 lg:w-[24rem]",
          inner && "hidden md:block",
        )}
      >
        <ConversationList activeId={activeId} onOpen={open} />
      </aside>
      <section className={cn("h-full min-w-0 flex-1", !inner && "hidden md:block")}>
        <Outlet />
      </section>
    </div>
  );
}
