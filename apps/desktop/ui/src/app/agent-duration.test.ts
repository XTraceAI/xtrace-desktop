import { describe, expect, it } from 'vitest';
import { agentDuration } from './agent-duration';

const MIN = 60_000;
const shown = (ms: number) => agentDuration(ms).visible;

describe('agentDuration', () => {
  it('states hours and the whole minutes that remain, spaced and unpadded', () => {
    expect(shown(192 * MIN)).toBe('3 h 12 m');
    expect(shown(124 * MIN)).toBe('2 h 4 m');
    expect(shown(47 * MIN)).toBe('0 h 47 m');
    expect(shown(60 * MIN)).toBe('1 h 0 m');
    expect(shown(120 * MIN)).toBe('2 h 0 m');
  });

  it('rounds to the nearest whole minute', () => {
    expect(shown(192.8 * MIN)).toBe('3 h 13 m');
    expect(shown(124.5 * MIN)).toBe('2 h 5 m');
    expect(shown(124.49 * MIN)).toBe('2 h 4 m');
    expect(shown(9.9 * MIN)).toBe('0 h 10 m');
  });

  it('rounds once and then splits, so the hour carries', () => {
    expect(shown(59.6 * MIN)).toBe('1 h 0 m');
    expect(shown(119.5 * MIN)).toBe('2 h 0 m');
    for (let ms = 0; ms <= 5 * 3_600_000; ms += 997) expect(shown(ms)).not.toMatch(/ 6\d m$/);
  });

  it('keeps a brief span apart from a measured zero', () => {
    expect(shown(1)).toBe('<1 m');
    expect(shown(29_999)).toBe('<1 m');
    expect(shown(30_000)).toBe('0 h 1 m');
    expect(shown(0)).toBe('0 h 0 m');
    expect(shown(-0)).toBe('0 h 0 m');
    expect(agentDuration(2_000).spoken).toBe('less than 1 minute');
  });

  it('states large durations with grouped hours', () => {
    expect(shown(720 * 3_600_000)).toBe('720 h 0 m');
    expect(shown(1_234 * 3_600_000 + 5.5 * MIN)).toBe('1,234 h 6 m');
  });

  it('says an impossible value is unmeasured', () => {
    expect(agentDuration(Number.NaN).spoken).toBe('not measured');
    expect(agentDuration(-1).spoken).toBe('not measured');
  });

  it('discloses the exact raw measurement, never a rounded one', () => {
    expect(agentDuration(11_568_000).exact).toBe('11,568,000 ms');
    expect(agentDuration(11_567_999).exact).toBe('11,567,999 ms');
    expect(agentDuration(2_000).exact).toBe('2,000 ms');
    expect(agentDuration(0).exact).toBe('0 ms');
    expect(agentDuration(11_568_000).spoken).toBe('3 hours 13 minutes');
    expect(agentDuration(60 * MIN).spoken).toBe('1 hour 0 minutes');
    expect(agentDuration(MIN).spoken).toBe('1 minute');
    expect(agentDuration(0).spoken).toBe('0 minutes');
  });
});
