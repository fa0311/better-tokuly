import { useQuery, useQueryClient } from "@tanstack/react-query";
import { type common, social } from "@tokuly/api";
import { type ReactNode, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Avatar } from "#/components/ui/avatar";
import { dopamineFactor } from "#/lib/appearance";
import { useLocations } from "#/stores/locations";
import { useMe } from "#/stores/session";
import { interactionClient } from "./data";
import { type MapEffect, onEffect } from "./effects";
import { useMap } from "./map-view";

const HEARTS = ["❤️", "💖", "💗", "💕"];
const CONFETTI = ["🎉", "✨", "🎊", "⭐", "💜", "💚"];
/** A recap plays at most this many reactions as animations; the rest are only counted. */
const RECAP_LIMIT = 12;

/** How lively script-driven motion may be: the ドパガキ度 factor, 0 when the device asks for reduced motion. */
function motionFactor(): number {
  if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) return 0;
  return dopamineFactor();
}

/** What flies for a reaction: glyphs, or the stamp's picture. */
function reactionGlyphs(reaction: social.MapReaction): {
  glyphs: string[];
  image?: string;
  label: string;
} {
  switch (reaction.kind) {
    case social.MapReactionKind.EYES:
      return { glyphs: ["👀"], label: "👀" };
    case social.MapReactionKind.DRINKS:
      return { glyphs: ["🍻", "🥤", "🧋"], label: "🍻" };
    case social.MapReactionKind.EMOJI:
      return { glyphs: [reaction.emoji || "✨"], label: reaction.emoji || "✨" };
    case social.MapReactionKind.STAMP:
      return {
        glyphs: ["✨"],
        image: reaction.stamp?.imageUrl,
        label: reaction.stamp?.name || "🖼️",
      };
    default:
      return { glyphs: HEARTS, label: "❤️" };
  }
}

interface Banner {
  id: number;
  user?: common.UserSummary;
  text: ReactNode;
  kind: string;
}

/**
 * Plays what happens on the map for a moment: reactions flying between pins,
 * knocks and buzzes shaking the map, confetti for a bump or an arrival — and
 * says each of them in words too, for people who switched motion off.
 * Reactions that arrived while the app was closed are fetched once and
 * replayed as a recap.
 */
export function EffectsLayer() {
  const { t } = useTranslation("map");
  const map = useMap();
  const me = useMe();
  const queryClient = useQueryClient();
  const overlay = useRef<HTMLDivElement>(null);
  const [banners, setBanners] = useState<Banner[]>([]);
  const nextId = useRef(1);
  const played = useRef(new Set<string>());

  const say = (banner: Omit<Banner, "id">, ms = 4000) => {
    const id = nextId.current++;
    setBanners((list) => [...list.slice(-3), { ...banner, id }]);
    setTimeout(() => setBanners((list) => list.filter((entry) => entry.id !== id)), ms);
  };

  /** Where a person is on screen right now; `null` when they are not on the map. */
  const screenPoint = (userId: string | undefined): { x: number; y: number } | null => {
    if (!userId) return null;
    const locations = useLocations.getState();
    const position = userId === me?.user?.id ? locations.own : locations.friends[userId]?.position;
    if (!position) return null;
    const point = map.project([position.lng, position.lat]);
    return { x: point.x, y: point.y - 44 };
  };

  /** The middle of the part of the map nothing covers. */
  const visibleCentre = () => {
    const padding = map.getPadding();
    const box = map.getContainer().getBoundingClientRect();
    return {
      x: ((padding.left ?? 0) + box.width - (padding.right ?? 0)) / 2,
      y: ((padding.top ?? 0) + box.height - (padding.bottom ?? 0)) / 2,
    };
  };

  const fly = (
    glyphs: string[],
    image: string | undefined,
    from: { x: number; y: number },
    to: { x: number; y: number },
    intensity: number,
  ) => {
    const layer = overlay.current;
    const factor = motionFactor();
    if (!layer || factor === 0) return;
    const count = Math.round((5 + intensity * 4) * Math.min(1.4, factor));
    for (let i = 0; i < count; i++) {
      const particle =
        image && i % 3 === 0
          ? Object.assign(document.createElement("img"), { src: image, alt: "" })
          : document.createElement("span");
      if (!(particle instanceof HTMLImageElement))
        particle.textContent = glyphs[i % glyphs.length] as string;
      const size =
        (image && particle instanceof HTMLImageElement ? 56 : 26) +
        intensity * 4 +
        Math.random() * 10;
      particle.style.cssText = `position:absolute;left:0;top:0;font-size:${size}px;width:${particle instanceof HTMLImageElement ? `${size}px` : "auto"};line-height:1;will-change:transform,opacity;pointer-events:none`;
      layer.append(particle);
      const spread = 40 + intensity * 14;
      const midX = (from.x + to.x) / 2 + (Math.random() - 0.5) * spread * 3;
      const midY = Math.min(from.y, to.y) - 60 - Math.random() * spread * 2;
      const endX = to.x + (Math.random() - 0.5) * spread;
      const endY = to.y + (Math.random() - 0.5) * spread;
      const spin = (Math.random() - 0.5) * 80;
      const animation = particle.animate(
        [
          {
            transform: `translate(${from.x}px, ${from.y}px) translate(-50%, -50%) scale(0.2) rotate(0deg)`,
            opacity: 0,
          },
          {
            transform: `translate(${midX}px, ${midY}px) translate(-50%, -50%) scale(1.1) rotate(${spin}deg)`,
            opacity: 1,
            offset: 0.55,
          },
          {
            transform: `translate(${endX}px, ${endY}px) translate(-50%, -50%) scale(1) rotate(${spin / 2}deg)`,
            opacity: 1,
            offset: 0.85,
          },
          {
            transform: `translate(${endX}px, ${endY - 30}px) translate(-50%, -50%) scale(1.6) rotate(${spin / 2}deg)`,
            opacity: 0,
          },
        ],
        {
          duration: (1300 + Math.random() * 500) * Math.max(0.6, factor),
          delay: i * 55,
          easing: "cubic-bezier(0.22, 1, 0.36, 1)",
          fill: "both",
        },
      );
      animation.onfinish = animation.oncancel = () => particle.remove();
    }
  };

  const confetti = (at: { x: number; y: number }) => {
    const layer = overlay.current;
    const factor = motionFactor();
    if (!layer || factor === 0) return;
    for (let i = 0; i < Math.round(22 * Math.min(1.4, factor)); i++) {
      const particle = document.createElement("span");
      particle.textContent = CONFETTI[i % CONFETTI.length] as string;
      particle.style.cssText =
        "position:absolute;left:0;top:0;font-size:24px;line-height:1;will-change:transform,opacity;pointer-events:none";
      layer.append(particle);
      const angle = Math.random() * 2 * Math.PI;
      const reach = 80 + Math.random() * 160;
      const animation = particle.animate(
        [
          {
            transform: `translate(${at.x}px, ${at.y}px) translate(-50%, -50%) scale(0.3)`,
            opacity: 1,
          },
          {
            transform: `translate(${at.x + Math.cos(angle) * reach}px, ${at.y + Math.sin(angle) * reach - 40}px) translate(-50%, -50%) scale(1.2) rotate(${Math.random() * 360}deg)`,
            opacity: 1,
            offset: 0.6,
          },
          {
            transform: `translate(${at.x + Math.cos(angle) * reach * 1.1}px, ${at.y + Math.sin(angle) * reach + 60}px) translate(-50%, -50%) scale(0.8) rotate(${Math.random() * 720}deg)`,
            opacity: 0,
          },
        ],
        {
          duration: 1400 * Math.max(0.6, factor),
          easing: "cubic-bezier(0.22, 1, 0.36, 1)",
          fill: "both",
        },
      );
      animation.onfinish = animation.oncancel = () => particle.remove();
    }
  };

  const shake = (times: number, strength: number) => {
    const factor = motionFactor();
    const frame = overlay.current?.closest<HTMLElement>("[data-map-frame]");
    if (!frame || factor === 0) return;
    const keyframes: Keyframe[] = [{ transform: "translate(0, 0)" }];
    for (let i = 0; i < times * 2; i++)
      keyframes.push({
        transform: `translate(${(i % 2 ? -1 : 1) * strength}px, ${(i % 2 ? 1 : -1) * strength * 0.4}px)`,
      });
    keyframes.push({ transform: "translate(0, 0)" });
    frame.animate(keyframes, {
      duration: Math.min(2400, 220 * times + 200),
      easing: "ease-in-out",
    });
  };

  const playReaction = (
    reaction: social.MapReaction,
    direction: "incoming" | "outgoing",
    recap = false,
  ) => {
    const { glyphs, image, label } = reactionGlyphs(reaction);
    const other = direction === "incoming" ? reaction.from?.id : reaction.toUserId;
    const mine = screenPoint(me?.user?.id) ?? visibleCentre();
    const box = map.getContainer().getBoundingClientRect();
    // Someone who is not on the map throws from (or is thrown at) the top edge.
    const theirs = screenPoint(other) ?? { x: box.width / 2, y: -20 };
    fly(
      glyphs,
      image,
      direction === "incoming" ? theirs : mine,
      direction === "incoming" ? mine : theirs,
      Math.min(5, Math.max(1, reaction.intensity)),
    );
    if (direction === "incoming" && !recap)
      say({
        kind: "reaction",
        user: reaction.from,
        text: t("reactions.received", { name: reaction.from?.displayName, reaction: label }),
      });
  };

  useEffect(
    () =>
      onEffect((effect: MapEffect) => {
        switch (effect.kind) {
          case "reaction":
            if (effect.direction === "incoming") {
              if (played.current.has(effect.reaction.id)) return;
              played.current.add(effect.reaction.id);
              // Seen live: it must not come back in the next recap.
              void interactionClient()
                .ackMapReactions({ reactionIds: [effect.reaction.id] })
                .catch(() => undefined);
            }
            playReaction(effect.reaction, effect.direction, effect.recap);
            break;
          case "knock":
            shake(Math.min(8, effect.count), 7);
            say({
              kind: "knock",
              user: effect.from,
              text: t("reactions.knocked", { name: effect.from.displayName, count: effect.count }),
            });
            break;
          case "buzz":
            navigator.vibrate?.([120, 60, 120, 60, 240]);
            shake(10, 12);
            say(
              {
                kind: "buzz",
                user: effect.from,
                text: t("reactions.buzzed", { name: effect.from.displayName }),
              },
              6000,
            );
            break;
          case "bump":
            // The one who shook second hears of the bump twice: from the call and from the push.
            if (played.current.has(effect.id)) return;
            played.current.add(effect.id);
            navigator.vibrate?.(200);
            confetti(screenPoint(me?.user?.id) ?? visibleCentre());
            say(
              {
                kind: "bump",
                user: effect.with,
                text: effect.placeName
                  ? t("reactions.bumpedAt", {
                      name: effect.with.displayName,
                      count: effect.pairCount,
                      place: effect.placeName,
                    })
                  : t("reactions.bumped", {
                      name: effect.with.displayName,
                      count: effect.pairCount,
                    }),
              },
              6000,
            );
            break;
          case "celebrate": {
            const point = effect.at ? map.project([effect.at.lng, effect.at.lat]) : visibleCentre();
            confetti({ x: point.x, y: point.y });
            break;
          }
        }
      }),
    [],
  );

  // Reactions that arrived while away: once per visit, as a recap.
  const { data: pending } = useQuery({
    queryKey: ["social", "pendingMapReactions"],
    queryFn: async () => (await interactionClient().listPendingMapReactions({})).reactions,
    staleTime: Number.POSITIVE_INFINITY,
    gcTime: 0,
  });
  useEffect(() => {
    const fresh = (pending ?? []).filter((reaction) => !played.current.has(reaction.id));
    if (fresh.length === 0) return;
    for (const reaction of fresh) played.current.add(reaction.id);
    const senders = [
      ...new Map(fresh.map((reaction) => [reaction.from?.id, reaction.from])).values(),
    ].filter((user): user is common.UserSummary => Boolean(user));
    say(
      {
        kind: "recap",
        user: senders[0],
        text:
          senders.length > 1
            ? t("reactions.recapMany", {
                name: senders[0]?.displayName,
                others: senders.length - 1,
                count: fresh.length,
              })
            : t("reactions.recap", { name: senders[0]?.displayName, count: fresh.length }),
      },
      7000,
    );
    const timers = fresh
      .slice(-RECAP_LIMIT)
      .map((reaction, index) =>
        setTimeout(() => playReaction(reaction, "incoming", true), 600 + index * 450),
      );
    void interactionClient()
      .ackMapReactions({ reactionIds: fresh.map((reaction) => reaction.id) })
      .then(() => queryClient.removeQueries({ queryKey: ["social", "pendingMapReactions"] }))
      .catch(() => undefined);
    return () => timers.forEach(clearTimeout);
  }, [pending]);

  return (
    <>
      <div
        ref={overlay}
        className="pointer-events-none absolute inset-0 z-40 overflow-hidden"
        aria-hidden
      />
      <div
        className="pointer-events-none absolute inset-x-0 top-24 z-40 flex flex-col items-center gap-2 px-4 md:top-6"
        aria-live="polite"
      >
        {banners.map((banner) => (
          <p
            key={banner.id}
            role="status"
            data-testid={`effect-${banner.kind}`}
            className="glass sticker flex max-w-full animate-pop-in items-center gap-2 rounded-full py-1.5 pr-4 pl-1.5 text-sm font-black"
          >
            {banner.user && <Avatar user={banner.user} size="sm" />}
            <span className="min-w-0 truncate">{banner.text}</span>
          </p>
        ))}
      </div>
    </>
  );
}
