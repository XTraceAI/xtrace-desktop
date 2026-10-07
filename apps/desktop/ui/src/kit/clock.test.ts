import { afterEach, expect, it, vi } from 'vitest';
import { calendarDay, clock, dayRange, isInstant } from './clock';

afterEach(() => {
  vi.unstubAllGlobals();
});

const at = Date.parse('2026-10-05T21:30:00Z');

/** Stand in another default locale, as a Mac set to it would give the page. */
function systemLocale(locale: string) {
  class SystemFormat extends Intl.DateTimeFormat {
    constructor(_locales?: Intl.LocalesArgument, options?: Intl.DateTimeFormatOptions) {
      super(locale, options);
    }
  }
  vi.stubGlobal('Intl', Object.assign(Object.create(Intl), { DateTimeFormat: SystemFormat }));
}

it('writes an instant in the system locale and its own 12- or 24-hour clock', () => {
  const utc = { timeZone: 'UTC' };
  expect(clock(at, utc)).toBe('9:30 PM');
  expect(clock(at, { ...utc, date: 'day' })).toBe('Oct 5, 9:30 PM');
  expect(clock(at, { ...utc, date: 'year' })).toBe('Oct 5, 2026, 9:30 PM');
  expect(clock(at, { ...utc, date: 'weekday' })).toBe('Mon 9:30 PM');
  expect(clock(at, { ...utc, date: 'day', time: false })).toBe('Oct 5');
  expect(clock(at, { ...utc, seconds: true })).toBe('9:30:00 PM');
  // The same calls on a Mac set to a 24-hour locale: nothing here forces one.
  systemLocale('en-GB');
  expect(clock(at, utc)).toBe('21:30');
  expect(clock(at, { ...utc, date: 'day' })).toBe('5 Oct, 21:30');
});

it('keeps a report zone it is given, and this Mac’s zone otherwise', () => {
  expect(clock(at, { timeZone: 'Asia/Tokyo' })).toBe('6:30 AM');
  expect(clock(at, { timeZone: 'America/New_York' })).toBe('5:30 PM');
});

it('drops :00 only from a whole hour on a 12-hour clock', () => {
  const eight = Date.parse('2026-10-05T20:00:00Z');
  expect(clock(eight, { timeZone: 'UTC', shortHour: true })).toBe('8 PM');
  expect(clock(at, { timeZone: 'UTC', shortHour: true })).toBe('9:30 PM');
  systemLocale('en-GB');
  expect(clock(eight, { timeZone: 'UTC', shortHour: true })).toBe('20:00');
});

it('writes plain spaces, whatever space the locale puts before AM or PM', () => {
  expect(clock(at, { timeZone: 'UTC' })).not.toMatch(/[\u202f\u2009\u00a0]/);
});

it('writes a report day and a span of days the way a date is written elsewhere', () => {
  expect(calendarDay('2026-09-14')).toBe('Sep 14');
  expect(calendarDay('2026-09-14', 'weekday-day')).toBe('Mon, Sep 14');
  expect(calendarDay('not-a-date')).toBe('not-a-date');
  expect(
    dayRange(Date.parse('2026-09-29T00:00:00Z'), Date.parse('2026-10-05T23:59:59Z'), 'UTC'),
  ).toMatch(/^Sep 29\s–\sOct 5, 2026$/);
});

it('says which stored instants can be written at all', () => {
  expect(isInstant(at)).toBe(true);
  expect(isInstant(Number.NaN)).toBe(false);
  expect(isInstant(8.64e15 + 1)).toBe(false);
});
