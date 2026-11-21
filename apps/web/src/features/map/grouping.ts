import { distanceMeters } from "#/lib/format";

export interface Positioned {
  id: string;
  lat: number;
  lng: number;
}

export interface Group<T extends Positioned> {
  members: T[];
  /** Centre of the members. */
  lat: number;
  lng: number;
}

/**
 * Groups points that are within `radiusMeters` of each other, transitively
 * (A near B and B near C puts all three together) — the "N人が集合中!" rule.
 * Every point ends up in exactly one group; loners form groups of one.
 */
export function groupByProximity<T extends Positioned>(points: T[], radiusMeters: number): Group<T>[] {
  const parent = points.map((_, index) => index);
  const find = (i: number): number => {
    while (parent[i] !== i) {
      parent[i] = parent[parent[i]!]!;
      i = parent[i]!;
    }
    return i;
  };
  for (let a = 0; a < points.length; a++) {
    for (let b = a + 1; b < points.length; b++) {
      if (distanceMeters(points[a]!, points[b]!) <= radiusMeters) parent[find(a)] = find(b);
    }
  }
  const buckets = new Map<number, T[]>();
  points.forEach((point, index) => {
    const root = find(index);
    const bucket = buckets.get(root);
    if (bucket) bucket.push(point);
    else buckets.set(root, [point]);
  });
  return [...buckets.values()].map((members) => ({
    members,
    lat: members.reduce((sum, m) => sum + m.lat, 0) / members.length,
    lng: members.reduce((sum, m) => sum + m.lng, 0) / members.length,
  }));
}
