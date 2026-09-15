import type { SessionPage } from './generated/SessionPage';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { DataSource } from './DataSource';
import type { AppInfo } from './generated/AppInfo';
import type { DbCounts } from './generated/DbCounts';
import type { NativeIndexStatus } from './generated/NativeIndexStatus';
import { commands, type DataEvent } from './ipc-names';

export class TauriDataSource implements DataSource {
  readonly kind = 'native';
  sessionsList(search: string, host: string | null, after: string | null) {
    return invoke<SessionPage>(commands.sessionsList, { search, host, after });
  }
  appInfo() {
    return invoke<AppInfo>(commands.appInfo);
  }
  dbCounts() {
    return invoke<DbCounts>(commands.dbCounts);
  }
  nativeIndexStatus() {
    return invoke<NativeIndexStatus>(commands.nativeIndexStatus);
  }
  subscribe(event: DataEvent, listener: () => void) {
    return listen(event, listener);
  }
}
