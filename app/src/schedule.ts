import type { PlanWindow } from "./types";

export const DAYS = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"] as const;
export const DAY_LABEL: Record<string, string> = { mon: "Mon", tue: "Tue", wed: "Wed", thu: "Thu", fri: "Fri", sat: "Sat", sun: "Sun" };

export const ACTIONS = [
  { id: "", label: "Do nothing" },
  { id: "sleep", label: "Put the computer to sleep" },
  { id: "hibernate", label: "Hibernate the computer" },
  { id: "shutdown", label: "Shut the computer down" },
] as const;

export function actionName(action: string | null): string {
  return ACTIONS.find((a) => a.id === (action ?? ""))?.label ?? String(action);
}

/** "07:30" and "7:30" are fine; anything else is not a time of day. */
export function normalTime(text: string): string | null {
  const m = /^\s*(\d{1,2}):(\d{2})\s*$/.exec(text);
  if (!m) return null;
  const [h, min] = [Number(m[1]), Number(m[2])];
  if (h > 23 || min > 59) return null;
  return `${String(h).padStart(2, "0")}:${String(min).padStart(2, "0")}`;
}

/** The window from what was typed, or the reason it cannot be used. */
export function windowFrom(days: string[], start: string, end: string, limitKb: string): PlanWindow | string {
  const s = normalTime(start);
  const e = normalTime(end);
  if (!s || !e) return "Times must look like 23:00.";
  if (s === e) return "A window cannot start and end at the same time.";
  const t = limitKb.trim().replace(",", ".");
  if (t !== "" && !/^\d+(\.\d+)?$/.test(t)) return "The speed limit must be a number.";
  // Seven days is the same as every day.
  const chosen = days.length === 7 ? [] : DAYS.filter((d) => days.includes(d));
  return { days: [...chosen], start: s, end: e, speedLimit: t === "" ? null : Math.round(Number(t) * 1024) };
}

/** "Mon, Tue 23:00–06:00" for a list. */
export function describeWindow(w: PlanWindow): string {
  const days = w.days.length === 0 ? "Every day" : w.days.map((d) => DAY_LABEL[d] ?? d).join(", ");
  const limit = w.speedLimit ? `, up to ${Math.round(w.speedLimit / 1024)} KB/s` : "";
  return `${days} ${w.start}–${w.end}${limit}`;
}
