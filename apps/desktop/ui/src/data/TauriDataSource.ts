import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { DataSource } from './DataSource';
import type { AppInfo } from './generated/AppInfo';
import type { DbCounts } from './generated/DbCounts';
import { commands, type DataEvent } from './ipc-names';

export class TauriDataSource implements DataSource {
  readonly kind = 'native';
  appInfo() {
    return invoke<AppInfo>(commands.appInfo);
  }
  dbCounts() {
    return invoke<DbCounts>(commands.dbCounts);
  }
  subscribe(event: DataEvent, listener: () => void) {
    return listen(event, listener);
  }
}
