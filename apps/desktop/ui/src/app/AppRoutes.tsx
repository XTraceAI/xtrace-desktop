import { SessionsPage } from './SessionsPage';
import { SessionDetailPage } from './session-detail/SessionDetailPage';
import { DashboardPage } from './dashboard/DashboardPage';
import { lazy, Suspense, useEffect, useRef } from 'react';
import { Route, Routes, useLocation } from 'react-router';
import { Shell } from './Shell';
import { PlaceholderPage } from './PlaceholderPage';
import { PrsPage } from './PrsPage';
import { RulebookPage } from './rulebook/RulebookPage';
import { SettingsPage } from './SettingsPage';
import { TrayPage } from './tray/TrayPage';
import { pages } from './routes';
import { StartupLanding } from './welcome/StartupLanding';
import { WelcomePage } from './welcome/WelcomePage';
const Gallery =
  import.meta.env.DEV && import.meta.env.VITE_GALLERY === '1'
    ? lazy(() => import('../gallery/Gallery'))
    : null;

export function AppRoutes() {
  // Only the address the app opened at is a startup: once any other address
  // has been shown, `/` is an ordinary link to Dashboard.
  const { pathname } = useLocation();
  const departed = useRef(false);
  useEffect(() => {
    if (pathname !== '/') departed.current = true;
  }, [pathname]);
  return (
    <Routes>
      {Gallery && (
        <Route
          path="/gallery"
          element={
            <Suspense fallback={<p>Opening gallery…</p>}>
              <Gallery />
            </Suspense>
          }
        />
      )}
      {/* The menu-bar popover's window: no Shell around it. */}
      <Route path="/tray" element={<TrayPage />} />
      <Route element={<Shell />}>
        <Route index element={<StartupLanding startup={!departed.current} />} />
        {pages.map((page) => (
          <Route
            key={page.path}
            path={page.path}
            element={
              page.path === '/sessions' ? (
                <SessionsPage />
              ) : page.path === '/sessions/:sessionId' ? (
                <SessionDetailPage />
              ) : page.path === '/dashboard' ? (
                <DashboardPage />
              ) : page.path === '/prs' ? (
                <PrsPage />
              ) : page.path === '/rulebook' ? (
                <RulebookPage view="overview" />
              ) : page.path === '/rulebook/fires' ? (
                <RulebookPage view="fires" />
              ) : page.path === '/rulebook/:ruleId' ? (
                <RulebookPage view="rule" />
              ) : page.path === '/settings' ? (
                <SettingsPage />
              ) : page.path === '/first-launch' ? (
                <WelcomePage />
              ) : (
                <PlaceholderPage title={page.title} />
              )
            }
          />
        ))}
        <Route path="*" element={<PlaceholderPage title="Page not found" />} />
      </Route>
    </Routes>
  );
}
