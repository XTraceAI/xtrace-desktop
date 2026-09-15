import { SessionsPage } from './SessionsPage';
import { lazy, Suspense } from 'react';
import { Navigate, Route, Routes } from 'react-router';
import { Shell } from './Shell';
import { PlaceholderPage } from './PlaceholderPage';
import { pages } from './routes';
const Gallery =
  import.meta.env.DEV && import.meta.env.VITE_GALLERY === '1'
    ? lazy(() => import('../gallery/Gallery'))
    : null;

export function AppRoutes() {
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
      <Route element={<Shell />}>
        <Route index element={<Navigate to="/dashboard" replace />} />
        {pages.map((page) => (
          <Route
            key={page.path}
            path={page.path}
            element={
              page.path === '/sessions' ? <SessionsPage /> : <PlaceholderPage title={page.title} />
            }
          />
        ))}
        <Route path="*" element={<PlaceholderPage title="Page not found" />} />
      </Route>
    </Routes>
  );
}
