/**
 * Keeps what a realtime push changed while a snapshot was being read over
 * HTTP, so the snapshot does not undo it.
 *
 * A feature reads again once its subscription is in effect (see
 * `SubscribeOptions.onSubscribed`). The answer to that read may have been
 * made on the server before some frame that already reached the page: taking
 * it as it is would drop a chat line pushed meanwhile, or bring back one that
 * was deleted meanwhile. Instead, every update a push makes is also recorded
 * here while a read is in flight, and replayed on the snapshot when it lands.
 * The updates must tolerate being applied to state that already has them
 * (add by ID, keep the newer sequence), which pushed updates do anyway.
 *
 * Replaying beats merging the snapshot into what the page holds: a line
 * deleted while the connection was down is gone from the snapshot and stays
 * gone, which a merge would not know.
 */
export interface PushesDuringRead<S> {
  /** Applies a pushed update to what is held now (through `apply`) and keeps it for reads in flight. */
  record(update: (state: S) => S): void;
  /**
   * Starts a read. `finish` applies to the snapshot every update recorded
   * since, in order; a read that is abandoned calls `cancel`.
   */
  begin(): { finish(snapshot: S): S; cancel(): void };
}

export function pushesDuringRead<S>(): PushesDuringRead<S> {
  const reads = new Set<((state: S) => S)[]>();
  return {
    record(update) {
      for (const updates of reads) updates.push(update);
    },
    begin() {
      const updates: ((state: S) => S)[] = [];
      reads.add(updates);
      return {
        finish(snapshot) {
          reads.delete(updates);
          return updates.reduce((state, update) => update(state), snapshot);
        },
        cancel() {
          reads.delete(updates);
        },
      };
    },
  };
}

/** One {@link PushesDuringRead} per key (a stream, a room), dropped when its last read ends. */
export function pushesDuringReadByKey<S>() {
  const byKey = new Map<string, { journal: PushesDuringRead<S>; reads: number }>();
  return {
    record(key: string, update: (state: S) => S): void {
      byKey.get(key)?.journal.record(update);
    },
    begin(key: string) {
      let entry = byKey.get(key);
      if (!entry) {
        entry = { journal: pushesDuringRead<S>(), reads: 0 };
        byKey.set(key, entry);
      }
      const held = entry;
      held.reads++;
      const read = held.journal.begin();
      let ended = false;
      const done = () => {
        if (ended) return;
        ended = true;
        held.reads--;
        if (held.reads === 0 && byKey.get(key) === held) byKey.delete(key);
      };
      return {
        finish(snapshot: S): S {
          done();
          return read.finish(snapshot);
        },
        cancel(): void {
          done();
          read.cancel();
        },
      };
    },
  };
}
