import { isTauri } from '@tauri-apps/api/core';
import { getCurrentWindow } from '@tauri-apps/api/window';
import {
  createContext,
  useContext,
  useEffect,
  useLayoutEffect,
  useState,
  type HTMLAttributes,
  type ReactNode,
} from 'react';

export type Theme = 'dark' | 'light';
export type ThemePreference = Theme | 'system';
const storageKey = 'xt.theme';
const mediaQuery = '(prefers-color-scheme: dark)';

function readPreference(): ThemePreference {
  try {
    const value = localStorage.getItem(storageKey);
    return value === 'light' || value === 'dark' ? value : 'system';
  } catch {
    return 'system';
  }
}

function systemTheme(): Theme {
  return typeof matchMedia === 'function'
    ? matchMedia(mediaQuery).matches
      ? 'dark'
      : 'light'
    : 'dark';
}

interface ThemeContextValue {
  theme: Theme;
  preference: ThemePreference;
  setPreference: (preference: ThemePreference) => void;
  toggle: () => void;
}

const ThemeContext = createContext<ThemeContextValue | null>(null);
const SurfaceThemeContext = createContext<Theme | undefined>(undefined);

/** Carry a scoped palette through React portals without copying DOM styles. */
export function useSurfaceTheme(): Theme | undefined {
  const scope = useContext(SurfaceThemeContext);
  const app = useContext(ThemeContext);
  return scope ?? app?.theme;
}

export function ThemeProvider({ children }: { children: ReactNode }) {
  const [preference, updatePreference] = useState(readPreference);
  const [system, setSystem] = useState(systemTheme);
  const theme = preference === 'system' ? system : preference;

  useLayoutEffect(() => {
    document.documentElement.dataset.theme = theme;
  }, [theme]);

  useEffect(() => {
    if (isTauri()) {
      // Native sidebar material must use the same appearance as the web content.
      void getCurrentWindow()
        .setTheme(preference === 'system' ? null : preference)
        .catch(() => {});
    }
  }, [preference]);

  useEffect(() => {
    let active = true;
    let unlisten: (() => void) | undefined;
    const sync = () => {
      if (active) setSystem(systemTheme());
    };
    const media = typeof matchMedia === 'function' ? matchMedia(mediaQuery) : undefined;
    media?.addEventListener('change', sync);
    sync();
    if (isTauri()) {
      const nativeWindow = getCurrentWindow();
      // Webview media is authoritative; native events only request a fresh read.
      void nativeWindow.theme().then(sync, () => {});
      void nativeWindow.onThemeChanged(sync).then(
        (stop) => {
          if (active) unlisten = stop;
          else stop();
        },
        () => {},
      );
    }
    return () => {
      active = false;
      media?.removeEventListener('change', sync);
      unlisten?.();
    };
  }, []);

  function setPreference(value: ThemePreference) {
    updatePreference(value);
    try {
      localStorage.setItem(storageKey, value);
    } catch {
      // The appearance still changes when storage is unavailable.
    }
  }

  return (
    <ThemeContext.Provider
      value={{
        theme,
        preference,
        setPreference,
        toggle: () => setPreference(theme === 'dark' ? 'light' : 'dark'),
      }}
    >
      {children}
    </ThemeContext.Provider>
  );
}

export function useTheme(): ThemeContextValue {
  const value = useContext(ThemeContext);
  if (!value) throw new Error('useTheme must be used inside ThemeProvider.');
  return value;
}

export function ThemeScope({ theme, ...props }: HTMLAttributes<HTMLDivElement> & { theme: Theme }) {
  return (
    <SurfaceThemeContext.Provider value={theme}>
      <div {...props} data-theme={theme} />
    </SurfaceThemeContext.Provider>
  );
}
