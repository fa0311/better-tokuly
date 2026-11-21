import { type Timestamp, timestampDate, timestampFromDate } from "@bufbuild/protobuf/wkt";

/** Protobuf timestamp → `Date`; `undefined` stays `undefined`. */
export function toDate(timestamp: Timestamp | undefined): Date | undefined {
  return timestamp ? timestampDate(timestamp) : undefined;
}

/** Protobuf timestamp → epoch milliseconds (0 when unset). */
export function toMillis(timestamp: Timestamp | undefined): number {
  return timestamp ? Number(timestamp.seconds) * 1000 + Math.floor(timestamp.nanos / 1e6) : 0;
}

export function fromDate(date: Date): Timestamp {
  return timestampFromDate(date);
}

export type { Timestamp };
