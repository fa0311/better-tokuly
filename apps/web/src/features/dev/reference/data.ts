import { useQuery } from "@tanstack/react-query";
import { create } from "zustand";
import { API_PATH } from "#/lib/api";
import { type dev, keys, referenceClient } from "../api";

/** The whole API as the server describes it: every service with its methods. */
export function useServices() {
  return useQuery({
    queryKey: keys.services,
    queryFn: async () => (await referenceClient().listServices({})).services,
    // It changes when the server is deployed, not while someone reads it.
    staleTime: 30 * 60_000,
    gcTime: 60 * 60_000,
  });
}

/** One method with the JSON Schemas of its request and response. */
export function useMethod(path: string) {
  return useQuery({
    queryKey: keys.method(path),
    queryFn: async () => (await referenceClient().getMethod({ path })).method,
    staleTime: 30 * 60_000,
  });
}

/** `/tokuly.notes.v1.NoteService/CreateNote` from the two parts of a reference address. */
export const methodPath = (service: string, method: string) => `/${service}/${method}`;

/** Where a method is called: the web origin, the API prefix, the method's path. */
export const methodUrl = (path: string) => `${window.location.origin}${API_PATH}${path}`;

/** `tokuly.notes.v1.NoteService` → `NoteService`. */
export const serviceShortName = (service: string) => service.slice(service.lastIndexOf(".") + 1);

export interface DomainGroup {
  domain: string;
  services: dev.ApiService[];
}

/** The services by domain, domains and services in alphabetical order. */
export function byDomain(services: readonly dev.ApiService[]): DomainGroup[] {
  const groups = new Map<string, dev.ApiService[]>();
  for (const service of services)
    groups.set(service.domain, [...(groups.get(service.domain) ?? []), service]);
  return [...groups.entries()]
    .sort(([a], [b]) => a.localeCompare(b))
    .map(([domain, list]) => ({
      domain,
      services: list.sort((a, b) => a.name.localeCompare(b.name)),
    }));
}

/** What a search needs to know about a method (the fields of `ApiMethod` it reads). */
export type SearchableMethod = Pick<dev.ApiMethod, "name" | "service" | "description" | "scope">;

/**
 * The methods a search finds, best first: every word of the query must occur
 * in the method's name, service, scopes or description; a hit in the name
 * counts most, then the service, then the rest.
 */
export function searchMethods<T extends SearchableMethod>(
  methods: readonly T[],
  query: string,
  limit = 60,
): T[] {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  if (words.length === 0) return [];
  const scored: { method: T; score: number }[] = [];
  for (const method of methods) {
    const name = method.name.toLowerCase();
    const service = method.service.toLowerCase();
    const rest = `${method.scope} ${method.description}`.toLowerCase();
    let score = 0;
    for (const word of words) {
      if (name === word) score += 12;
      else if (name.startsWith(word)) score += 8;
      else if (name.includes(word)) score += 5;
      else if (service.includes(word)) score += 3;
      else if (rest.includes(word)) score += 1;
      else {
        score = -1;
        break;
      }
    }
    if (score > 0) scored.push({ method, score });
  }
  return scored
    .sort((a, b) => b.score - a.score || a.method.name.localeCompare(b.method.name))
    .slice(0, limit)
    .map((entry) => entry.method);
}

interface ReferenceState {
  /** What is typed into the search box (shared by the sidebar and the phone page). */
  query: string;
  setQuery(query: string): void;
}

export const useReference = create<ReferenceState>((set) => ({
  query: "",
  setQuery: (query) => set({ query }),
}));
