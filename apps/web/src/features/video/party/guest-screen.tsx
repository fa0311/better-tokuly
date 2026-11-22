import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Link, Navigate } from "@tanstack/react-router";
import { isNotFound, type video } from "@tokuly/api";
import { PartyPopper } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { TextField } from "#/components/ui/field";
import { LogoMark } from "#/components/ui/logo";
import { EmptyState } from "#/components/ui/page";
import { PaneSpinner } from "#/components/ui/spinner";
import { errorMessage } from "#/lib/errors";
import { useSessionStatus } from "#/stores/session";
import { partyClient, videoKeys } from "../api";
import { PartyRoomView } from "./party-room";
import { PartyGone, PartyPreviewCard, usePartyPreview } from "./party-screen";

const NAME_KEY = "tokuly.party.guestName";
const tokenKey = (code: string) => `tokuly.party.guest.${code}`;

function remembered(key: string): string {
  try {
    return window.sessionStorage.getItem(key) ?? window.localStorage.getItem(key) ?? "";
  } catch {
    return "";
  }
}

/**
 * `/join/party/<code>`: the page a party's link opens, for anyone. People
 * with an account are passed on to the room inside the app; others see what
 * is playing and — when the host allows guests — join with just a name.
 * A guest's seat survives a reload of the tab.
 */
export function PartyGuestScreen({ code }: { code: string }) {
  const { t } = useTranslation("video");
  const status = useSessionStatus();
  const queryClient = useQueryClient();
  const preview = usePartyPreview(code);
  const [name, setName] = useState(() => remembered(NAME_KEY));
  const [seat, setSeat] = useState<{ session: video.PartySession; token: string } | null>(null);
  const [left, setLeft] = useState(false);

  const join = useMutation({
    mutationFn: (input: { guestName: string; guestToken: string }) =>
      partyClient().joinParty({ code, ...input }),
    onSuccess: ({ session }, input) => {
      if (!session) return;
      const token = session.guestToken || input.guestToken;
      try {
        window.sessionStorage.setItem(tokenKey(code), token);
        if (input.guestName) window.localStorage.setItem(NAME_KEY, input.guestName);
      } catch {
        // Private mode: the seat lasts until the tab is reloaded.
      }
      setLeft(false);
      setSeat({ session, token });
    },
    onError: (error, input) => {
      // A remembered seat that is gone is no reason to complain: the form is still there.
      if (input.guestName) toast.error(errorMessage(error));
      try {
        window.sessionStorage.removeItem(tokenKey(code));
      } catch {
        // Nothing was stored.
      }
    },
  });

  // A reload of the tab takes the same seat again.
  const rejoin = join.mutate;
  useEffect(() => {
    if (status !== "signedOut") return;
    const token = remembered(tokenKey(code));
    if (token) rejoin({ guestName: "", guestToken: token });
  }, [status, code, rejoin]);

  if (status === "signedIn")
    return <Navigate to="/party/$partyId" params={{ partyId: code }} replace />;

  if (seat && !left) {
    return (
      <div className="h-dvh bg-bg">
        <PartyRoomView
          key={seat.session.self?.id}
          session={seat.session}
          guestToken={seat.token}
          standalone
          onRejoin={() =>
            join.mutate({ guestName: seat.session.self?.name ?? name, guestToken: "" })
          }
          onLeft={() => {
            try {
              window.sessionStorage.removeItem(tokenKey(code));
            } catch {
              // Nothing was stored.
            }
            void queryClient.invalidateQueries({ queryKey: videoKeys.partyPreview(code) });
            setLeft(true);
          }}
        />
      </div>
    );
  }

  const party = preview.data?.party;
  const trimmed = name.trim();
  return (
    <main className="flex min-h-dvh flex-col bg-bg">
      <header className="flex h-16 items-center gap-3 px-4">
        <Link to="/" aria-label="Tokuly">
          <LogoMark className="size-10" />
        </Link>
        <span className="flex-1" />
        <Link
          to="/login"
          search={{ redirect: `/party/${code}` }}
          className="pressable sticker-sm inline-flex h-10 items-center rounded-full bg-surface px-4 text-sm font-bold"
        >
          {t("party.guest.signIn")}
        </Link>
      </header>
      <div className="grid flex-1 place-items-center p-4">
        {status === "unknown" || preview.isPending || (join.isPending && !trimmed) ? (
          <PaneSpinner />
        ) : preview.isError || !party ? (
          preview.isError && !isNotFound(preview.error) ? (
            <EmptyState emoji="😵‍💫" title={errorMessage(preview.error)} />
          ) : (
            <PartyGone ended={false} />
          )
        ) : party.endedAt ? (
          <PartyGone ended />
        ) : (
          <PartyPreviewCard preview={preview.data}>
            {left && (
              <p className="mb-3 text-sm font-bold text-ink-soft">{t("party.guest.left")}</p>
            )}
            {party.allowGuests ? (
              <form
                className="flex flex-col gap-3 text-left"
                onSubmit={(event) => {
                  event.preventDefault();
                  if (trimmed) join.mutate({ guestName: trimmed, guestToken: "" });
                }}
              >
                <TextField
                  label={t("party.guest.name")}
                  value={name}
                  maxLength={24}
                  autoComplete="nickname"
                  placeholder={t("party.guest.namePlaceholder")}
                  hint={t("party.guest.nameHint")}
                  onChange={(event) => setName(event.target.value)}
                />
                <Button
                  type="submit"
                  size="lg"
                  block
                  icon={<PartyPopper className="size-5" />}
                  disabled={!trimmed}
                  loading={join.isPending}
                >
                  {t("party.guest.join")}
                </Button>
              </form>
            ) : (
              <>
                <p className="text-sm leading-relaxed text-ink-soft">
                  {t("party.guest.membersOnly")}
                </p>
                <Link
                  to="/login"
                  search={{ redirect: `/party/${code}` }}
                  className="pressable sticker mt-4 inline-flex h-12 items-center rounded-full bg-brand px-6 font-bold text-on-brand"
                >
                  {t("party.guest.signInToJoin")}
                </Link>
              </>
            )}
          </PartyPreviewCard>
        )}
      </div>
    </main>
  );
}
