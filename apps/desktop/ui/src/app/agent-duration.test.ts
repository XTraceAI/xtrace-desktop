import { describe, expect, it } from 'vitest';
import { hours } from '../kit/format';
import { agentDuration } from './agent-duration';

const MIN = 60_000;
const shown = (ms: number) => agentDuration(ms).visible;

describe('agentDuration', () => {
  it('states whole minutes as hours and two-digit remaining minutes', () => {
    expect(shown(192 * MIN)).toBe('3h12m');
    expect(shown(124 * MIN)).toBe('2h04m');
    expect(shown(47 * MIN)).toBe('0h47m');
    expect(shown(23 * MIN)).toBe('0h23m');
    expect(shown(60 * MIN)).toBe('1h00m');
    expect(shown(120 * MIN)).toBe('2h00m');
  });

  it('keeps the tenth of a minute the column already showed', () => {
    expect(shown(192.8 * MIN)).toBe('3h12.8m');
    expect(shown(124.5 * MIN)).toBe('2h04.5m');
    expect(shown(0.1 * MIN)).toBe('0h00.1m');
    expect(shown(0.05 * MIN)).toBe('0h00.1m');
    expect(shown(9.9 * MIN)).toBe('0h09.9m');
    // Rounded to the tenth, not truncated to a whole minute.
    expect(shown(47.96 * MIN)).toBe('0h48m');
    expect(shown(47.94 * MIN)).toBe('0h47.9m');
  });

  it('rounds once and then splits, so the hour carries', () => {
    // The column read "60" for these; they read one hour, never 0h60m.
    expect(shown(59.96 * MIN)).toBe('1h00m');
    expect(shown(3_597_600)).toBe('1h00m');
    expect(shown(119.95 * MIN)).toBe('2h00m');
    expect(shown(59.94 * MIN)).toBe('0h59.9m');
    for (let ms = 0; ms <= 5 * 3_600_000; ms += 997) expect(shown(ms)).not.toMatch(/h6\d/);
  });

  it('shows the same total tenths the one-decimal scale shows, ties included', () => {
    for (const ms of [3_000, 9_000, 27_000, 33_000, 1_380_000, 11_568_000, 3_597_000]) {
      const minutes = ms / MIN;
      const [, h, m] = /^([\d,]+)h([\d.]+)m$/.exec(shown(ms)) ?? [];
      const total = Number(h.replace(/,/g, '')) * 60 + Number(m);
      expect(Number(hours(minutes).replace(/,/g, ''))).toBeCloseTo(total, 9);
    }
  });

  it('keeps a brief span apart from a measured zero', () => {
    expect(shown(2_000)).toBe('<0.1m');
    expect(shown(1)).toBe('<0.1m');
    expect(shown(2_999)).toBe('<0.1m');
    expect(shown(0)).toBe('0h00m');
    expect(shown(-0)).toBe('0h00m');
    expect(agentDuration(2_000).spoken).toBe('less than 0.1 minutes');
  });

  it('states large durations with grouped hours', () => {
    expect(shown(720 * 3_600_000)).toBe('720h00m');
    expect(shown(1_234 * 3_600_000 + 5.5 * MIN)).toBe('1,234h05.5m');
  });

  it('discloses the exact raw measurement, never a rounded one', () => {
    expect(agentDuration(11_568_000).exact).toBe('11,568,000 ms');
    expect(agentDuration(11_567_999).exact).toBe('11,567,999 ms');
    expect(agentDuration(2_000).exact).toBe('2,000 ms');
    expect(agentDuration(0).exact).toBe('0 ms');
    expect(agentDuration(11_568_000).spoken).toBe('3 hours 12.8 minutes');
    expect(agentDuration(60 * MIN).spoken).toBe('1 hour 0 minutes');
    expect(agentDuration(MIN).spoken).toBe('1 minute');
    expect(agentDuration(0).spoken).toBe('0 minutes');
  });
});
