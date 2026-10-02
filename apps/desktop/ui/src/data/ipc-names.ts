export const commands = {
  dashboard: 'metrics_dashboard',
  tokensByHost: 'tokens_by_host',
  sessionsList: 'sessions_list',
  appInfo: 'app_info',
  dbCounts: 'db_counts',
  nativeIndexStatus: 'native_index_status',
} as const;
export const events = {
  importReceived: 'ingest://import-received',
  backfillProgress: 'ingest://backfill-progress',
  hostConnected: 'ingest://host-connected',
  turnCompleted: 'ingest://turn-completed',
  fireReceived: 'rulebook://fire-received',
  prsRefreshed: 'prs://refreshed',
  nativeIndexStatus: 'native-index://status',
} as const;
export type DataEvent = (typeof events)[keyof typeof events];
