import { Navigate } from 'react-router';
import { useData } from '../../data/DataProvider';
import { useAppInfo } from '../useAppInfo';
import { welcomeCompleted } from './welcome-completion';

/**
 * The default address at startup. Welcome is offered only to a first launch:
 * nobody has continued past it here, and the database held no indexed
 * sessions before this process's index started (live counts would race the
 * initial scan). Any other entry, and a later visit to `/`, opens Dashboard.
 */
export function StartupLanding({ startup }: { startup: boolean }) {
  const { source } = useData();
  const info = useAppInfo();
  if (!startup || source.kind === 'preview') return <Navigate to="/dashboard" replace />;
  // The Shell reports reading and failure; an unknown fact keeps Dashboard.
  if (info.isPending) return null;
  const welcome =
    info.isSuccess && !info.data.had_indexed_history_at_startup && !welcomeCompleted();
  return <Navigate to={welcome ? '/first-launch' : '/dashboard'} replace />;
}
