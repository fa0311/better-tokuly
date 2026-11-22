import { type Timestamp, toDate } from "@tokuly/api";
import i18n from "#/lib/i18n";

/** "2026/11/22 14:05": the timestamp format of every console table. */
export function formatDateTime(timestamp: Timestamp | Date | undefined): string {
  const date = timestamp instanceof Date ? timestamp : toDate(timestamp);
  if (!date) return "—";
  return new Intl.DateTimeFormat(i18n.language, {
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  }).format(date);
}

export function formatDate(timestamp: Timestamp | Date | undefined): string {
  const date = timestamp instanceof Date ? timestamp : toDate(timestamp);
  if (!date) return "—";
  return new Intl.DateTimeFormat(i18n.language, {
    year: "numeric",
    month: "short",
    day: "numeric",
  }).format(date);
}

/** Whole numbers with grouping; the API sends counters as 64-bit integers. */
export function formatCount(value: bigint | number): string {
  return value.toLocaleString(i18n.language);
}

/** 1,284 / 12.9K / 4.2M: for tiles where the exact figure is in the tooltip or table. */
export function formatCompact(value: bigint | number): string {
  return new Intl.NumberFormat(i18n.language, {
    notation: "compact",
    maximumFractionDigits: 1,
  }).format(value);
}

const BYTE_UNITS = ["B", "KB", "MB", "GB", "TB"] as const;

export function formatBytes(bytes: bigint | number): string {
  let value = Number(bytes);
  let unit = 0;
  while (value >= 1024 && unit < BYTE_UNITS.length - 1) {
    value /= 1024;
    unit++;
  }
  return `${unit === 0 || value >= 100 ? Math.round(value) : value.toFixed(1)} ${BYTE_UNITS[unit]}`;
}

/** The moment a `datetime-local` value names, or `undefined` when it is empty or malformed. */
export function fromLocalInput(value: string): Date | undefined {
  if (!value) return undefined;
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? undefined : date;
}
