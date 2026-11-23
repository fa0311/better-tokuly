import type { community } from "@tokuly/api";
import { createContext, useContext } from "react";

export const CommunityContext = createContext<community.Community | null>(null);

/** The community whose page this is (inside `/g/<handle>/…`). */
export function useCurrentCommunity(): community.Community {
  const value = useContext(CommunityContext);
  if (!value) throw new Error("useCurrentCommunity needs a community page around it");
  return value;
}
