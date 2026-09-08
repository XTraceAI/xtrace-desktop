import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { StrictMode } from 'react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { isTauri } from '@tauri-apps/api/core';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { ThemeProvider, ThemeScope, useTheme } from './ThemeProvider';

vi.mock('@tauri-apps/api/core', () => ({ isTauri: vi.fn() }));
vi.mock('@tauri-apps/api/window', () => ({ getCurrentWindow: vi.fn() }));
let dark = true;
let media: EventTarget;

function Controls() {
  const { preference, theme, setPreference, toggle } = useTheme();
  return (
    <>
      <output>
        {preference}:{theme}
      </output>
      <button onClick={toggle}>Toggle</button>
      <button onClick={() => setPreference('light')}>Light</button>
      <button onClick={() => setPreference('system')}>System</button>
      <ThemeScope theme="dark" data-testid="scope">
        Scoped
      </ThemeScope>
    </>
  );
}
function mount() {
  return render(
    <ThemeProvider>
      <Controls />
    </ThemeProvider>,
  );
}
function changeSystem(value: boolean) {
  act(() => {
    dark = value;
    media.dispatchEvent(new Event('change'));
  });
}
beforeEach(() => {
  vi.resetAllMocks();
  localStorage.clear();
  dark = true;
  media = new EventTarget();
  Object.defineProperty(media, 'matches', { get: () => dark });
  vi.stubGlobal('matchMedia', () => media);
});
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

it('defaults to system and follows live media without overriding a saved preference', () => {
  mount();
  expect(screen.getByText('system:dark')).toBeTruthy();
  changeSystem(false);
  expect(document.documentElement.dataset.theme).toBe('light');
  fireEvent.click(screen.getByText('Light'));
  changeSystem(true);
  expect(screen.getByText('light:light')).toBeTruthy();
  fireEvent.click(screen.getByText('System'));
  expect(screen.getByText('system:dark')).toBeTruthy();
});

it('persists a toggle across remount and keeps ThemeScope forced', () => {
  const first = mount();
  fireEvent.click(screen.getByText('Toggle'));
  expect(localStorage.getItem('xt.theme')).toBe('light');
  expect(screen.getByTestId('scope').dataset.theme).toBe('dark');
  first.unmount();
  mount();
  expect(screen.getByText('light:light')).toBeTruthy();
  fireEvent.click(screen.getByText('Toggle'));
  expect(document.documentElement.dataset.theme).toBe('dark');
});

it('survives denied storage, invalid preferences, and absent media support', () => {
  localStorage.setItem('xt.theme', 'invalid');
  const first = mount();
  expect(screen.getByText('system:dark')).toBeTruthy();
  first.unmount();
  vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => {
    throw new Error('denied');
  });
  vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
    throw new Error('denied');
  });
  vi.stubGlobal('matchMedia', undefined);
  mount();
  fireEvent.click(screen.getByText('Light'));
  expect(document.documentElement.dataset.theme).toBe('light');
});

it('uses native events only as media nudges and disposes late StrictMode subscriptions', async () => {
  vi.mocked(isTauri).mockReturnValue(true);
  const stops = [vi.fn(), vi.fn()];
  const completions: ((stop: () => void) => void)[] = [];
  const nudges: (() => void)[] = [];
  vi.mocked(getCurrentWindow).mockReturnValue({
    theme: vi.fn().mockResolvedValue('light'),
    onThemeChanged: vi.fn((nudge: () => void) => {
      nudges.push(nudge);
      return new Promise((resolve) => completions.push(resolve));
    }),
  } as unknown as ReturnType<typeof getCurrentWindow>);
  const app = render(
    <StrictMode>
      <ThemeProvider>
        <Controls />
      </ThemeProvider>
    </StrictMode>,
  );
  await act(async () => {
    completions[0](stops[0]);
    completions[1](stops[1]);
  });
  expect(stops[0]).toHaveBeenCalledOnce();
  expect(screen.getByText('system:dark')).toBeTruthy();
  act(() => {
    dark = false;
    nudges[1]();
  });
  expect(screen.getByText('system:light')).toBeTruthy();
  app.unmount();
  expect(stops[1]).toHaveBeenCalledOnce();
});

it('continues with media when native theme access rejects', async () => {
  vi.mocked(isTauri).mockReturnValue(true);
  vi.mocked(getCurrentWindow).mockReturnValue({
    theme: () => Promise.reject(new Error('unavailable')),
    onThemeChanged: () => Promise.reject(new Error('unavailable')),
  } as unknown as ReturnType<typeof getCurrentWindow>);
  await act(async () => {
    mount();
  });
  changeSystem(false);
  expect(screen.getByText('system:light')).toBeTruthy();
});
