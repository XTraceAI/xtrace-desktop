import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import {
  CHECK_TIMEOUT,
  DOWNLOAD_TIMEOUT,
  UPDATE_INTERVAL,
  UpdateController,
  type UpdateResource,
} from './update-controller';

const controllers: UpdateController[] = [];
function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: Error) => void;
  const promise = new Promise<T>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
}
function setup() {
  const resource: UpdateResource = {
    version: '0.2.0',
    download: vi.fn().mockResolvedValue(undefined),
    install: vi.fn().mockResolvedValue(undefined),
    close: vi.fn().mockResolvedValue(undefined),
  };
  const backend = {
    enabled: vi.fn().mockResolvedValue(true),
    check: vi.fn().mockResolvedValue(resource),
    restart: vi.fn().mockResolvedValue(undefined),
  };
  const controller = new UpdateController(backend);
  controllers.push(controller);
  return { resource, backend, controller };
}
beforeEach(() => vi.useFakeTimers());
afterEach(() => {
  controllers.splice(0).forEach((controller) => controller.dispose());
  vi.useRealTimers();
});
const flush = async () => {
  await vi.advanceTimersByTimeAsync(0);
};

it('downloads on startup with timeouts; never installs or restarts automatically', async () => {
  const { controller, backend, resource } = setup();
  controller.start();
  await flush();
  expect(backend.check).toHaveBeenCalledWith({ timeout: CHECK_TIMEOUT });
  expect(resource.download).toHaveBeenCalledWith(expect.any(Function), {
    timeout: DOWNLOAD_TIMEOUT,
  });
  expect(controller.snapshot()).toEqual({ phase: 'ready', version: '0.2.0' });
  expect(resource.install).not.toHaveBeenCalled();
  expect(backend.restart).not.toHaveBeenCalled();
});

it('does no check when the native build gate is disabled', async () => {
  const { controller, backend } = setup();
  backend.enabled.mockResolvedValue(false);
  controller.start();
  await flush();
  await vi.advanceTimersByTimeAsync(UPDATE_INTERVAL);
  expect(controller.snapshot()).toEqual({ phase: 'disabled' });
  expect(backend.check).not.toHaveBeenCalled();
  expect(backend.enabled).toHaveBeenCalledTimes(1);
});

it('reports progress but only offers restart after verified download resolves', async () => {
  const { controller, resource } = setup();
  const transfer = deferred<void>();
  vi.mocked(resource.download).mockImplementation(async (progress) => {
    progress({ event: 'Started', data: { contentLength: 100 } });
    progress({ event: 'Progress', data: { chunkLength: 50 } });
    progress({ event: 'Finished' });
    await transfer.promise;
  });
  const pending = controller.check();
  await flush();
  expect(controller.snapshot()).toEqual({
    phase: 'downloading',
    version: '0.2.0',
    downloaded: 50,
    total: 100,
  });
  await controller.restartToUpdate();
  expect(resource.install).not.toHaveBeenCalled();
  transfer.resolve();
  await pending;
  expect(controller.snapshot().phase).toBe('ready');
});

it('signature/download failure closes the resource and makes installation unavailable until a successful retry', async () => {
  const { controller, resource, backend } = setup();
  vi.mocked(resource.download).mockRejectedValueOnce(new Error('invalid signature'));
  await controller.check();
  expect(controller.snapshot()).toMatchObject({ phase: 'error', retry: 'check' });
  expect(resource.close).toHaveBeenCalledTimes(1);
  await controller.restartToUpdate();
  expect(resource.install).not.toHaveBeenCalled();
  await controller.retry();
  expect(backend.check).toHaveBeenCalledTimes(2);
  expect(controller.snapshot().phase).toBe('ready');
});

it('handles failed checks and no update without installing', async () => {
  const { controller, backend, resource } = setup();
  backend.check.mockRejectedValueOnce(new Error('timeout')).mockResolvedValueOnce(null);
  await controller.check();
  expect(controller.snapshot()).toMatchObject({ phase: 'error', retry: 'check' });
  await controller.retry();
  expect(controller.snapshot().phase).toBe('idle');
  expect(resource.download).not.toHaveBeenCalled();
});

it('failed install retains the downloaded update and retries without downloading; restart follows success', async () => {
  const { controller, resource, backend } = setup();
  const order: string[] = [];
  vi.mocked(resource.install)
    .mockRejectedValueOnce(new Error('permission denied'))
    .mockImplementationOnce(async () => {
      order.push('install');
    });
  backend.restart.mockImplementation(async () => {
    order.push('restart');
  });
  await controller.check();
  await controller.restartToUpdate();
  expect(controller.snapshot()).toMatchObject({ phase: 'error', retry: 'install' });
  expect(resource.close).not.toHaveBeenCalled();
  expect(backend.restart).not.toHaveBeenCalled();
  await controller.retry();
  expect(order).toEqual(['install', 'restart']);
  expect(resource.download).toHaveBeenCalledTimes(1);
  expect(resource.close).toHaveBeenCalledTimes(1);
});

it('restart failure retries only restart, not installation', async () => {
  const { controller, backend, resource } = setup();
  backend.restart.mockRejectedValueOnce(new Error('IPC failure'));
  await controller.check();
  await controller.restartToUpdate();
  expect(controller.snapshot()).toMatchObject({ phase: 'error', retry: 'restart' });
  await controller.retry();
  expect(resource.install).toHaveBeenCalledTimes(1);
  expect(backend.restart).toHaveBeenCalledTimes(2);
});

it('serializes repeated install clicks and periodic checks during installation', async () => {
  const { controller, resource, backend } = setup();
  const installing = deferred<void>();
  vi.mocked(resource.install).mockReturnValue(installing.promise);
  controller.start();
  await flush();
  const pending = controller.restartToUpdate();
  await controller.restartToUpdate();
  await vi.advanceTimersByTimeAsync(UPDATE_INTERVAL);
  expect(resource.install).toHaveBeenCalledTimes(1);
  expect(backend.check).toHaveBeenCalledTimes(1);
  expect(backend.restart).not.toHaveBeenCalled();
  installing.resolve();
  await pending;
  expect(backend.restart).toHaveBeenCalledTimes(1);
});

it('checks every six hours but never replaces a ready download', async () => {
  const { controller, backend } = setup();
  backend.check.mockResolvedValueOnce(null);
  controller.start();
  await flush();
  await vi.advanceTimersByTimeAsync(UPDATE_INTERVAL - 1);
  expect(backend.check).toHaveBeenCalledTimes(1);
  await vi.advanceTimersByTimeAsync(1);
  expect(backend.check).toHaveBeenCalledTimes(2);
  await vi.advanceTimersByTimeAsync(UPDATE_INTERVAL * 2);
  expect(backend.check).toHaveBeenCalledTimes(2);
  expect(controller.snapshot().phase).toBe('ready');
});

it('StrictMode/remount observers do not duplicate pending checks or lose their completion', async () => {
  const { controller, backend, resource } = setup();
  const check = deferred<UpdateResource | null>();
  backend.check.mockReturnValue(check.promise);
  const oldObserver = vi.fn();
  const newObserver = vi.fn();
  const unsubscribe = controller.subscribe(oldObserver);
  controller.start();
  await flush();
  unsubscribe();
  oldObserver.mockClear();
  controller.subscribe(newObserver);
  controller.start();
  await controller.check();
  await vi.advanceTimersByTimeAsync(UPDATE_INTERVAL);
  expect(backend.check).toHaveBeenCalledTimes(1);
  check.resolve(resource);
  await flush();
  expect(oldObserver).not.toHaveBeenCalled();
  expect(newObserver).toHaveBeenCalled();
  expect(controller.snapshot().phase).toBe('ready');
});

it('disposes late check completion without downloading or notifying old views', async () => {
  const { controller, backend, resource } = setup();
  const check = deferred<UpdateResource | null>();
  backend.check.mockReturnValue(check.promise);
  const observer = vi.fn();
  controller.subscribe(observer);
  const pending = controller.check();
  await flush();
  controller.dispose();
  observer.mockClear();
  check.resolve(resource);
  await pending;
  expect(resource.close).toHaveBeenCalledTimes(1);
  expect(resource.download).not.toHaveBeenCalled();
  expect(observer).not.toHaveBeenCalled();
});

it('waits for pending download before disposing its resource; no stale ready state', async () => {
  const { controller, resource } = setup();
  const transfer = deferred<void>();
  vi.mocked(resource.download).mockReturnValue(transfer.promise);
  const pending = controller.check();
  await flush();
  controller.dispose();
  expect(resource.close).not.toHaveBeenCalled();
  transfer.resolve();
  await pending;
  expect(resource.close).toHaveBeenCalledTimes(1);
  expect(controller.snapshot().phase).toBe('downloading');
});

it('disposal during installation prevents late restart and releases the resource', async () => {
  const { controller, resource, backend } = setup();
  const installing = deferred<void>();
  vi.mocked(resource.install).mockReturnValue(installing.promise);
  await controller.check();
  const pending = controller.restartToUpdate();
  controller.dispose();
  installing.resolve();
  await pending;
  expect(backend.restart).not.toHaveBeenCalled();
  expect(resource.close).toHaveBeenCalledTimes(1);
});
