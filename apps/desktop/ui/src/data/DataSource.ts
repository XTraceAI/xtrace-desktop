import type { AppInfo } from './generated/AppInfo';
import type { DbCounts } from './generated/DbCounts';
import type { NativeIndexStatus } from './generated/NativeIndexStatus';
import type { DataEvent } from './ipc-names';

export type Unsubscribe = () => void;
/** Extend this seam with generated query DTOs when their owning screen lands. */
export interface DataSource {
  readonly kind: 'native' | 'fixture' | 'preview';
  appInfo(): Promise<AppInfo>;
  dbCounts(): Promise<DbCounts>;
  nativeIndexStatus(): Promise<NativeIndexStatus>;
  subscribe(event: DataEvent, listener: () => void): Promise<Unsubscribe>;
}
