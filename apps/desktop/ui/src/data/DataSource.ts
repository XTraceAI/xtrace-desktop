import type { DashboardMetrics } from './generated/DashboardMetrics';
import type { TokensByHost } from './generated/TokensByHost';
import type { EnvironmentMetrics } from './generated/EnvironmentMetrics';
import type { SessionPage } from './generated/SessionPage';
import type { AppInfo } from './generated/AppInfo';
import type { DbCounts } from './generated/DbCounts';
import type { NativeIndexStatus } from './generated/NativeIndexStatus';
import type { DataEvent } from './ipc-names';

export type Unsubscribe = () => void;
/** Extend this seam with generated query DTOs when their owning screen lands. */
export interface DataSource {
  dashboard(windowDays: number): Promise<DashboardMetrics>;
  tokensByHost(windowDays: number): Promise<TokensByHost>;
  /** Tool usage counts with an unknown inventory beside configured components; never recalculated here. */
  environment(windowDays: number): Promise<EnvironmentMetrics>;
  readonly kind: 'native' | 'fixture' | 'preview';
  sessionsList(search: string, host: string | null, after: string | null): Promise<SessionPage>;
  appInfo(): Promise<AppInfo>;
  dbCounts(): Promise<DbCounts>;
  nativeIndexStatus(): Promise<NativeIndexStatus>;
  subscribe(event: DataEvent, listener: () => void): Promise<Unsubscribe>;
}
