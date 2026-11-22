import { addDays } from "./dates";

/** One calendar event for an `.ics` file: on a day (`date`) or at a time (`start`). */
export interface IcsEvent {
  /** Stable across exports, so importing twice updates instead of duplicating. */
  uid: string;
  title: string;
  /** All-day event: `YYYY-MM-DD` (and optionally the last day it covers). */
  date?: string;
  endDate?: string;
  /** Timed event. Without `end` it lasts an hour. */
  start?: Date;
  end?: Date;
  description?: string;
  location?: string;
  url?: string;
}

function escapeText(text: string): string {
  return text.replace(/\\/g, "\\\\").replace(/;/g, "\\;").replace(/,/g, "\\,").replace(/\r?\n/g, "\\n");
}

/** Lines longer than 75 bytes continue on the next line after a space (RFC 5545). */
function fold(line: string): string {
  const encoder = new TextEncoder();
  if (encoder.encode(line).length <= 75) return line;
  const parts: string[] = [];
  let current = "";
  let size = 0;
  for (const char of line) {
    const bytes = encoder.encode(char).length;
    // Continuation lines start with a space, which counts.
    if (size + bytes > (parts.length === 0 ? 75 : 74)) {
      parts.push(current);
      current = "";
      size = 0;
    }
    current += char;
    size += bytes;
  }
  parts.push(current);
  return parts.join("\r\n ");
}

function utcStamp(date: Date): string {
  return `${date.toISOString().replace(/[-:]/g, "").slice(0, 15)}Z`;
}

const compactDay = (day: string) => day.replace(/-/g, "");

/** The text of an `.ics` calendar holding `events`. */
export function buildIcs(events: IcsEvent[], now: Date = new Date()): string {
  const lines = ["BEGIN:VCALENDAR", "VERSION:2.0", "PRODID:-//Tokuly//Plans//JA", "CALSCALE:GREGORIAN", "METHOD:PUBLISH"];
  for (const event of events) {
    if (!event.date && !event.start) continue;
    lines.push("BEGIN:VEVENT", `UID:${event.uid}@tokuly`, `DTSTAMP:${utcStamp(now)}`);
    if (event.start) {
      lines.push(`DTSTART:${utcStamp(event.start)}`, `DTEND:${utcStamp(event.end ?? new Date(event.start.getTime() + 3_600_000))}`);
    } else if (event.date) {
      // An all-day event ends on the day after its last day.
      lines.push(`DTSTART;VALUE=DATE:${compactDay(event.date)}`, `DTEND;VALUE=DATE:${compactDay(addDays(event.endDate ?? event.date, 1))}`);
    }
    lines.push(`SUMMARY:${escapeText(event.title)}`);
    if (event.description) lines.push(`DESCRIPTION:${escapeText(event.description)}`);
    if (event.location) lines.push(`LOCATION:${escapeText(event.location)}`);
    if (event.url) lines.push(`URL:${event.url}`);
    lines.push("END:VEVENT");
  }
  lines.push("END:VCALENDAR");
  return `${lines.map(fold).join("\r\n")}\r\n`;
}

/** Hands the person a calendar file to keep or open in their calendar app. */
export function downloadIcs(filename: string, text: string): void {
  const url = URL.createObjectURL(new Blob([text], { type: "text/calendar;charset=utf-8" }));
  const link = document.createElement("a");
  link.href = url;
  link.download = filename;
  document.body.append(link);
  link.click();
  link.remove();
  setTimeout(() => URL.revokeObjectURL(url), 10_000);
}
