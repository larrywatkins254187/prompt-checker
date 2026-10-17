import type { UsageRange } from "../api";

export type RangePreset = "today" | "week" | "30d" | "90d" | "365d" | "custom";

export const PRESETS: { id: RangePreset; label: string }[] = [
  { id: "today", label: "Today" },
  { id: "week", label: "This week" },
  { id: "30d", label: "30 days" },
  { id: "90d", label: "90 days" },
  { id: "365d", label: "1 year" },
  { id: "custom", label: "Custom" },
];

function startOfDay(d: Date): Date {
  return new Date(d.getFullYear(), d.getMonth(), d.getDate());
}
function addDays(d: Date, n: number): Date {
  return new Date(d.getFullYear(), d.getMonth(), d.getDate() + n);
}

/** `YYYY-MM-DD` in local time (value format of <input type="date">). */
export function isoDay(d: Date): string {
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
}

export function parseDay(s: string): Date {
  const [y, m, d] = s.split("-").map(Number);
  return new Date(y, m - 1, d);
}

/** Local [start, end) for a preset; weeks start on Monday. Custom days are inclusive. */
export function presetBounds(preset: RangePreset, custom?: { from: string; to: string }): { start: Date; end: Date } {
  const today = startOfDay(new Date());
  const tomorrow = addDays(today, 1);
  switch (preset) {
    case "today":
      return { start: today, end: tomorrow };
    case "week": {
      const sinceMonday = (today.getDay() + 6) % 7;
      return { start: addDays(today, -sinceMonday), end: tomorrow };
    }
    case "30d":
      return { start: addDays(today, -29), end: tomorrow };
    case "90d":
      return { start: addDays(today, -89), end: tomorrow };
    case "365d":
      return { start: addDays(today, -364), end: tomorrow };
    case "custom": {
      const a = custom?.from ? parseDay(custom.from) : addDays(today, -6);
      const b = custom?.to ? parseDay(custom.to) : today;
      const [lo, hi] = a <= b ? [a, b] : [b, a];
      return { start: lo, end: addDays(hi, 1) };
    }
  }
}

export function toUsageRange(start: Date, end: Date): UsageRange {
  const days = Math.round((end.getTime() - start.getTime()) / 86_400_000);
  return {
    from: start.toISOString(),
    to: end.toISOString(),
    tzOffsetMinutes: -start.getTimezoneOffset(),
    bucket: days <= 1 ? "hour" : "day",
  };
}

/** Axis label for a bucket key (`YYYY-MM-DD` or `YYYY-MM-DDTHH`). */
export function bucketLabel(key: string): string {
  if (key.includes("T")) return `${key.slice(11, 13)}:00`;
  return parseDay(key).toLocaleDateString(undefined, { month: "short", day: "numeric" });
}

export function bucketTitle(key: string): string {
  if (key.includes("T")) {
    const d = parseDay(key.slice(0, 10));
    return `${d.toLocaleDateString(undefined, { weekday: "short", month: "short", day: "numeric" })}, ${key.slice(11, 13)}:00`;
  }
  return parseDay(key).toLocaleDateString(undefined, { weekday: "short", year: "numeric", month: "short", day: "numeric" });
}
