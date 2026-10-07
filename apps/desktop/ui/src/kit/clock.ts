/**
 * The one way this app writes a date or a clock time.
 *
 * Every screen states an instant in this Mac's own locale and its own 12- or
 * 24-hour preference: nothing here names a locale or an hour cycle, so a
 * reader who set a 24-hour clock sees `14:30` everywhere and one who did not
 * sees `2:30 PM` everywhere. A report that names its own time zone passes it
 * as `timeZone`; without one the instant is written in this Mac's zone.
 *
 * The day comes first, as a short month and day (`Oct 5`), with the year
 * (`Oct 5, 2026`), the weekday (`Mon`) or both weekday and day
 * (`Mon, Oct 5`) when asked for. Unusual spaces some locales put before
 * `AM`/`PM` are written as plain spaces, so the same text reads the same in
 * a label, a tooltip and a test.
 */
export interface ClockOptions {
  /** Which part of the date to state before the time; none by default. */
  date?: 'none' | 'day' | 'year' | 'weekday' | 'weekday-day';
  /** Whether to state the time of day; true by default. */
  time?: boolean;
  /** Whether to state seconds as well. */
  seconds?: boolean;
  /**
   * On a 12-hour clock only, `8 PM` instead of `8:00 PM` for a whole hour.
   * A 24-hour clock keeps `20:00` whole.
   */
  shortHour?: boolean;
  /** An IANA zone named by a report; this Mac's own zone when absent. */
  timeZone?: string;
}

const dateParts: Record<NonNullable<ClockOptions['date']>, Intl.DateTimeFormatOptions> = {
  none: {},
  day: { month: 'short', day: 'numeric' },
  year: { month: 'short', day: 'numeric', year: 'numeric' },
  weekday: { weekday: 'short' },
  'weekday-day': { weekday: 'short', month: 'short', day: 'numeric' },
};

/** The format options for one request; no locale and no hour cycle, ever. */
function formatOptions({
  date = 'none',
  time = true,
  seconds = false,
  timeZone,
}: ClockOptions): Intl.DateTimeFormatOptions {
  return {
    ...dateParts[date],
    ...(time && { hour: 'numeric', minute: '2-digit' }),
    ...(time && seconds && { second: '2-digit' }),
    timeZone,
  };
}

const plainSpaces = (text: string) => text.replace(/\s+/g, ' ');

/** Whether `ms` names an instant a Date can hold. */
export const isInstant = (ms: number) =>
  Number.isFinite(ms) && !Number.isNaN(new Date(ms).getTime());

/**
 * An instant in words. The caller checks `isInstant` first where a stored
 * value could be out of range; an unreadable instant here is a bug.
 */
export function clock(ms: number, options: ClockOptions = {}): string {
  // Made per call, not cached: a cached formatter would keep this Mac's old
  // time zone after the zone changes.
  const format = new Intl.DateTimeFormat(undefined, formatOptions(options));
  if (!options.shortHour) return plainSpaces(format.format(ms));
  const parts = format.formatToParts(ms);
  const twelveHour = parts.some((part) => part.type === 'dayPeriod');
  const minute = parts.find((part) => part.type === 'minute');
  const whole = twelveHour && minute?.value === '00' && !options.seconds;
  return plainSpaces(
    parts
      .filter(
        (part, index) =>
          !whole ||
          !(
            part.type === 'minute' ||
            (part.type === 'literal' && parts[index + 1]?.type === 'minute')
          ),
      )
      .map((part) => part.value)
      .join(''),
  );
}

/**
 * A local calendar date written as `YYYY-MM-DD` (already local to whatever
 * report sent it), as the date part of `clock` writes a day: `Oct 5`. A
 * string that is not such a date is returned as written.
 */
export function calendarDay(date: string, part: 'day' | 'weekday-day' = 'day'): string {
  const parsed = Date.parse(`${date}T00:00:00Z`);
  return Number.isFinite(parsed) && /^\d{4}-\d{2}-\d{2}$/.test(date)
    ? clock(parsed, { date: part, time: false, timeZone: 'UTC' })
    : date;
}

/** A span of days, `Sep 29 – Oct 5, 2026`, from its first to its last included instant. */
export function dayRange(startMs: number, lastMs: number, timeZone?: string): string {
  return new Intl.DateTimeFormat(undefined, {
    ...dateParts.year,
    timeZone,
  }).formatRange(startMs, Math.max(startMs, lastMs));
}
