import { invoke, isTauri } from '@tauri-apps/api/core';
import { useEffect, useState } from 'react';
import { useTheme, type ThemePreference } from './theme/ThemeProvider';

interface AppInfo {
  name: string;
  version: string;
}

type InfoState = { status: 'preview' | 'loading' | 'error' } | { status: 'ready'; info: AppInfo };

export function App() {
  const { preference, setPreference } = useTheme();
  const [native] = useState(isTauri);
  const nativeMac = native && navigator.platform.startsWith('Mac');
  const [revision, setRevision] = useState(0);
  const [state, setState] = useState<InfoState>({
    status: native ? 'loading' : 'preview',
  });

  useEffect(() => {
    if (!native) return;
    let active = true;
    void invoke<AppInfo>('app_info').then(
      (info) => {
        if (active) setState({ status: 'ready', info });
      },
      () => {
        if (active) setState({ status: 'error' });
      },
    );
    return () => {
      active = false;
    };
  }, [native, revision]);

  function refresh() {
    setState({ status: 'loading' });
    setRevision((value) => value + 1);
  }

  return (
    <div className={`app-shell${nativeMac ? ' native-mac' : ''}`}>
      <aside className="sidebar">
        {nativeMac && (
          <div className="window-chrome" data-tauri-drag-region="deep" aria-hidden="true" />
        )}
        <div className="brand" data-tauri-drag-region="deep">
          <img src="/mark.png" alt="" width="36" height="36" />
          <span>
            xtrace <small>desktop</small>
          </span>
        </div>
        <div className="sidebar-section">WORKSPACE</div>
        <div className="current-page" aria-current="page">
          <span className="home-glyph" aria-hidden="true">
            ⌂
          </span>
          Home
        </div>
        <div className="sidebar-footer">
          <span className="status-dot" />
          Foundation preview
        </div>
      </aside>
      <div className="main-shell">
        <header className="topbar" data-tauri-drag-region="deep">
          <span>
            Workspace <span className="breadcrumb-separator">/</span> <strong>Home</strong>
          </span>
          <div className="appearance" data-tauri-drag-region="false">
            <label htmlFor="appearance">Appearance</label>
            <select
              id="appearance"
              value={preference}
              onChange={(event) => setPreference(event.target.value as ThemePreference)}
            >
              <option value="system">System</option>
              <option value="dark">Dark</option>
              <option value="light">Light</option>
            </select>
            {native && (
              <button
                type="button"
                className="refresh-button"
                data-tauri-drag-region="false"
                onClick={refresh}
                disabled={state.status === 'loading'}
              >
                Refresh app info
              </button>
            )}
          </div>
        </header>
        <main>
          <div className="welcome">
            <img
              className="welcome-mark"
              src="/mark.png"
              alt="XTrace brand mark"
              width="76"
              height="76"
            />
            <p className="eyebrow">A FOUNDATION FOR WHAT COMES NEXT</p>
            <h1>Welcome to XTrace.</h1>
            <p className="intro">Your workspace for understanding how you build with AI.</p>
            <section className="build-card" aria-label="App information">
              <div className="card-heading">
                <span className="status-dot" />
                <h2>Desktop foundation</h2>
                <span className="build-badge">Preview</span>
              </div>
              <p>
                The app shell is ready. Session capture and analytics will arrive in later builds.
              </p>
              <div className="build-status" role="status" aria-live="polite">
                {state.status === 'preview' && (
                  <span>
                    Browser preview · Native app information is available in the desktop app.
                  </span>
                )}
                {state.status === 'loading' && <span>Reading native app information…</span>}
                {state.status === 'error' && (
                  <span>
                    App information could not be loaded. Use Refresh app info to try again.
                  </span>
                )}
                {state.status === 'ready' && (
                  <>
                    <span>{state.info.name}</span>
                    <span className="version">v{state.info.version}</span>
                  </>
                )}
              </div>
            </section>
          </div>
          <p className="footer-note">Built for your Mac. Starting with the essentials.</p>
        </main>
      </div>
    </div>
  );
}
