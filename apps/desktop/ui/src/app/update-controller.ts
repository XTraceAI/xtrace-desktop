import type { DownloadEvent } from '@tauri-apps/plugin-updater';

export const UPDATE_INTERVAL = 6 * 60 * 60 * 1000;
export const CHECK_TIMEOUT = 30_000;
export const DOWNLOAD_TIMEOUT = 10 * 60 * 1000;

export interface UpdateResource {
  version: string;
  download: (
    progress: (event: DownloadEvent) => void,
    options: { timeout: number },
  ) => Promise<void>;
  install: () => Promise<void>;
  close: () => Promise<void>;
}

export interface UpdateBackend {
  enabled: () => Promise<boolean>;
  check: (options: { timeout: number }) => Promise<UpdateResource | null>;
  restart: () => Promise<void>;
}

export type UpdateState =
  | { phase: 'idle' | 'checking' | 'disabled' }
  | { phase: 'downloading'; version: string; downloaded: number; total?: number }
  | { phase: 'ready' | 'installing' | 'installed'; version: string }
  | { phase: 'error'; message: string; retry: 'check' | 'install' | 'restart' };

/** One controller per renderer, independent of React mount lifetimes. */
export class UpdateController {
  private state: UpdateState = { phase: 'idle' };
  private listeners = new Set<() => void>();
  private resource: UpdateResource | null = null;
  private busy = false;
  private disposed = false;
  private started = false;
  private installed = false;
  private timer?: ReturnType<typeof setInterval>;

  constructor(private backend: UpdateBackend) {}

  snapshot = () => this.state;
  subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };

  private publish(state: UpdateState) {
    if (this.disposed) return;
    this.state = state;
    this.listeners.forEach((listener) => listener());
  }

  start() {
    if (this.started || this.disposed) return;
    this.started = true;
    void this.check();
    this.timer = setInterval(() => void this.check(), UPDATE_INTERVAL);
  }

  async check() {
    if (
      this.busy ||
      this.resource ||
      this.installed ||
      this.disposed ||
      this.state.phase === 'disabled'
    )
      return;
    this.busy = true;
    let candidate: UpdateResource | null = null;
    try {
      if (!(await this.backend.enabled())) {
        this.publish({ phase: 'disabled' });
        if (this.timer) clearInterval(this.timer);
        return;
      }
      if (this.disposed) return;
      this.publish({ phase: 'checking' });
      candidate = await this.backend.check({ timeout: CHECK_TIMEOUT });
      if (this.disposed) return;
      if (!candidate) {
        this.publish({ phase: 'idle' });
        return;
      }
      const version = candidate.version;
      let downloaded = 0;
      let total: number | undefined;
      this.publish({ phase: 'downloading', version, downloaded });
      await candidate.download(
        (event) => {
          if (event.event === 'Started') total = event.data.contentLength;
          if (event.event === 'Progress') downloaded += event.data.chunkLength;
          // Finished means transfer ended, not that signature verification passed.
          this.publish({ phase: 'downloading', version, downloaded, total });
        },
        { timeout: DOWNLOAD_TIMEOUT },
      );
      if (this.disposed) return;
      this.resource = candidate;
      candidate = null;
      this.publish({ phase: 'ready', version });
    } catch {
      this.publish({
        phase: 'error',
        message: 'The update could not be checked or downloaded safely. Your app is still usable.',
        retry: 'check',
      });
    } finally {
      if (candidate) await this.close(candidate);
      this.busy = false;
      if (this.disposed) await this.release();
    }
  }

  /** Called only by the explicit Restart to update button (or its retry). */
  async restartToUpdate() {
    if (this.busy || this.disposed || (!this.resource && !this.installed)) return;
    this.busy = true;
    try {
      if (!this.installed && this.resource) {
        this.publish({ phase: 'installing', version: this.resource.version });
        await this.resource.install();
        this.installed = true;
        this.publish({ phase: 'installed', version: this.resource.version });
        await this.release();
      }
      // An unmounted view does not dispose this controller. Explicit disposal
      // does prevent a late install result from requesting restart.
      if (!this.disposed) await this.backend.restart();
    } catch {
      this.publish({
        phase: 'error',
        message: this.installed
          ? 'The update was installed, but restart failed. Retry restarting the app.'
          : 'Update installation failed. Retry installation.',
        retry: this.installed ? 'restart' : 'install',
      });
    } finally {
      this.busy = false;
      if (this.disposed) await this.release();
    }
  }

  retry = () =>
    this.state.phase === 'error' && this.state.retry === 'check'
      ? this.check()
      : this.restartToUpdate();

  private async close(resource: UpdateResource) {
    try {
      await resource.close();
    } catch {
      /* Window teardown may already have released it. */
    }
  }

  private async release() {
    const resource = this.resource;
    this.resource = null;
    if (resource) await this.close(resource);
  }

  dispose() {
    this.disposed = true;
    if (this.timer) clearInterval(this.timer);
    this.listeners.clear();
    // Do not close resources while plugin download/install is using them.
    if (!this.busy) void this.release();
  }
}
