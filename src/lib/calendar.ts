// Google Calendar: types mirror src-tauri/src/calendar.rs.
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export interface CalendarInfo {
  id: string;
  name: string;
  color: string;
  primary: boolean;
  /** Shown in the island. */
  on: boolean;
}

export interface CalEvent {
  id: string;
  calendarId: string;
  title: string;
  /** RFC 3339 date-time, or YYYY-MM-DD for all-day events. */
  start: string;
  end: string;
  allDay: boolean;
  location: string | null;
  meet: string | null;
  link: string | null;
  color: string;
  declined: boolean;
}

export interface CalendarStatus {
  connected: boolean;
  hasClientId: boolean;
  hasSecret: boolean;
  signingIn: boolean;
  account: string | null;
  error: string | null;
  calendars: CalendarInfo[];
}

export interface Agenda {
  events: CalEvent[];
  fetchedAt: number;
}

export const calendarApi = {
  status: () => invoke<CalendarStatus>("calendar_status"),
  agenda: () => invoke<Agenda>("calendar_agenda"),
  setSecret: (secret: string) => invoke<void>("calendar_set_secret", { secret }),
  login: () => invoke<void>("calendar_login"),
  cancelLogin: () => invoke<void>("calendar_cancel_login"),
  logout: () => invoke<void>("calendar_logout"),
  refresh: () => invoke<void>("calendar_refresh"),
  open: (url: string) => invoke<void>("calendar_open", { url }),
};

export const onCalendarStatus = (cb: (s: CalendarStatus) => void): Promise<UnlistenFn> =>
  listen<CalendarStatus>("calendar-status", (e) => cb(e.payload));
export const onAgenda = (cb: (a: Agenda) => void): Promise<UnlistenFn> => listen<Agenda>("calendar-agenda", (e) => cb(e.payload));

/** An event with its times as local ms. All-day dates count from local midnight. */
export interface Timed extends CalEvent {
  startMs: number;
  endMs: number;
}

function parseWhen(s: string, allDay: boolean): number {
  if (allDay) {
    const [y, m, d] = s.split("-").map(Number);
    return new Date(y, m - 1, d).getTime();
  }
  return Date.parse(s);
}

export function dayStart(ms: number): number {
  const d = new Date(ms);
  return new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
}

/** Every fetched event with local times (past ones too), sorted; declined ones kept only when asked. */
export function allTimed(events: CalEvent[], showDeclined: boolean): Timed[] {
  return events
    .filter((e) => showDeclined || !e.declined)
    .map((e) => ({ ...e, startMs: parseWhen(e.start, e.allDay), endMs: parseWhen(e.end, e.allDay) }))
    .filter((e) => !Number.isNaN(e.startMs))
    .sort(byStart);
}

const byStart = (a: Timed, b: Timed) => Number(b.allDay) - Number(a.allDay) || a.startMs - b.startMs || a.title.localeCompare(b.title);

/** Events from today's midnight on, sorted, declined ones kept only when asked. */
export function timed(events: CalEvent[], now: number, showDeclined: boolean): Timed[] {
  const from = dayStart(now);
  return allTimed(events, showDeclined).filter((e) => e.endMs > from);
}

/** Midnight on the first of the month `delta` months from `ms`'s month. */
export function monthStart(ms: number, delta = 0): number {
  const d = new Date(ms);
  return new Date(d.getFullYear(), d.getMonth() + delta, 1).getTime();
}

/** The 6 × 7 days (Monday first) of the grid showing `month`. */
export function monthGrid(month: number): number[] {
  const first = new Date(month);
  const lead = (first.getDay() + 6) % 7;
  return Array.from({ length: 42 }, (_, i) => new Date(first.getFullYear(), first.getMonth(), 1 - lead + i).getTime());
}


/** The timed (not all-day, not declined) event to mention in the pill:
 *  starting within `minutes`, or started less than 5 minutes ago. */
export function soonEvent(events: Timed[], now: number, minutes: number): Timed | null {
  if (minutes <= 0) return null;
  return (
    events
      .filter((e) => !e.allDay && !e.declined && e.startMs - now <= minutes * 60_000 && now - e.startMs < 5 * 60_000 && e.endMs > now)
      .sort((a, b) => a.startMs - b.startMs)[0] ?? null
  );
}

/** "in 8 m", "now", "in 1 h 5 m". */
export function untilText(startMs: number, now: number): string {
  const mins = Math.ceil((startMs - now) / 60_000);
  if (mins <= 0) return "now";
  return mins >= 60 ? `in ${Math.floor(mins / 60)} h${mins % 60 ? ` ${mins % 60} m` : ""}` : `in ${mins} m`;
}

const timeFmt = new Intl.DateTimeFormat(undefined, { hour: "numeric", minute: "2-digit" });
export const clockTime = (ms: number) => timeFmt.format(ms);

export function dayLabel(dayMs: number, now: number): string {
  const diff = Math.round((dayMs - dayStart(now)) / 86_400_000);
  if (diff === 0) return "Today";
  if (diff === 1) return "Tomorrow";
  return new Intl.DateTimeFormat(undefined, { weekday: "long", day: "numeric", month: "short" }).format(dayMs);
}
