import { useLocation, useSearchParams } from 'react-router';

export function PlaceholderPage({ title }: { title: string }) {
  const { pathname } = useLocation();
  const [params] = useSearchParams();
  return (
    <section className="xt-placeholder">
      <h1>{title}</h1>
      <p className="xt-page-subline">This view is coming next.</p>
      {pathname === '/sessions' && params.has('pr') && (
        <p className="xt-page-context">Pull request filter: {params.get('pr')}</p>
      )}
    </section>
  );
}
