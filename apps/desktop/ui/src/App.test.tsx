import { invoke, isTauri } from '@tauri-apps/api/core';
import {
  act,
  cleanup,
  fireEvent,
  render as testingRender,
  screen,
  waitFor,
} from '@testing-library/react';
import { StrictMode, type ReactNode } from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { App } from './App';
import { ThemeProvider } from './theme/ThemeProvider';
function render(ui: ReactNode) {
  return testingRender(<ThemeProvider>{ui}</ThemeProvider>);
}
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({ theme: async () => 'dark', onThemeChanged: async () => () => {} }),
}));

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(), isTauri: vi.fn() }));

beforeEach(() => {
  vi.resetAllMocks();
  vi.mocked(isTauri).mockReturnValue(true);
  vi.spyOn(navigator, 'platform', 'get').mockReturnValue('MacIntel');
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe('desktop foundation', () => {
  it('renders the brand mark and real native build metadata', async () => {
    vi.mocked(invoke).mockResolvedValue({ name: 'XTrace Desktop', version: '0.1.0' });
    const { container } = render(<App />);
    expect(screen.getByRole('img', { name: 'XTrace brand mark' })).toBeTruthy();
    expect(await screen.findByText('v0.1.0')).toBeTruthy();
    expect(invoke).toHaveBeenCalledWith('app_info');
    expect(container.querySelector('.native-mac .window-chrome')).toBeTruthy();
    expect(
      screen
        .getByRole('button', { name: 'Refresh app info' })
        .getAttribute('data-tauri-drag-region'),
    ).toBe('false');
  });

  it('labels browser preview honestly and does not invoke native commands', () => {
    vi.mocked(isTauri).mockReturnValue(false);
    const { container } = render(<App />);
    expect(screen.getByText(/Browser preview/)).toBeTruthy();
    expect(invoke).not.toHaveBeenCalled();
    expect(container.querySelector('.window-chrome')).toBeNull();
    expect(screen.queryByRole('button', { name: 'Refresh app info' })).toBeNull();
  });

  it('shows an IPC failure and lets the user retry successfully', async () => {
    vi.mocked(invoke)
      .mockRejectedValueOnce(new Error('IPC unavailable'))
      .mockResolvedValueOnce({ name: 'XTrace Desktop', version: '0.1.0' });
    render(<App />);
    expect(await screen.findByText(/App information could not be loaded/)).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: 'Refresh app info' }));
    expect(await screen.findByText('v0.1.0')).toBeTruthy();
    expect(screen.queryByText(/App information could not be loaded/)).toBeNull();
    expect(invoke).toHaveBeenCalledTimes(2);
  });

  it('ignores an older IPC reply after the effect has been cleaned up', async () => {
    let completeOldRequest: (value: unknown) => void = () => {};
    vi.mocked(invoke)
      .mockImplementationOnce(
        () =>
          new Promise((resolve) => {
            completeOldRequest = resolve;
          }),
      )
      .mockResolvedValueOnce({ name: 'XTrace Desktop', version: '0.1.0' });
    const { unmount } = testingRender(
      <StrictMode>
        <ThemeProvider>
          <App />
        </ThemeProvider>
      </StrictMode>,
    );
    expect(await screen.findByText('v0.1.0')).toBeTruthy();
    await act(async () => {
      completeOldRequest({ name: 'Obsolete response', version: '0.0.0' });
    });
    await waitFor(() => expect(screen.queryByText('v0.0.0')).toBeNull());
    expect(screen.getByText('v0.1.0')).toBeTruthy();
    unmount();
  });
});
