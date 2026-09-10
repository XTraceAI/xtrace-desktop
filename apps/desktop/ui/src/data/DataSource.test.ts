import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { afterEach, expect, it, vi } from 'vitest';
import exported from '../../fixtures/F1.json';
import { createDataSource } from './createDataSource';
import { loadFixtureDataSource } from './FixtureDataSource';
import { commands, events } from './ipc-names';
vi.mock('@tauri-apps/api/core', () => ({ isTauri: vi.fn(), invoke: vi.fn() }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn() }));
afterEach(() => {
  vi.resetAllMocks();
  vi.unstubAllEnvs();
});

it('native presence always chooses real IPC, including when Vite asks for a fixture', async () => {
  vi.mocked(isTauri).mockReturnValue(true);
  vi.stubEnv('VITE_XTRACE_FIXTURE', 'F1');
  vi.mocked(invoke).mockImplementation(async (command) =>
    command === commands.appInfo ? exported.app_info : exported.db_counts,
  );
  const stop = vi.fn();
  vi.mocked(listen).mockResolvedValue(stop);
  const source = await createDataSource();
  expect(source.kind).toBe('native');
  expect(await source.appInfo()).toEqual(exported.app_info);
  expect(await source.dbCounts()).toEqual(exported.db_counts);
  expect(invoke).toHaveBeenNthCalledWith(1, 'app_info');
  expect(invoke).toHaveBeenNthCalledWith(2, 'db_counts');
  const listener = vi.fn();
  const unlisten = await source.subscribe(events.importReceived, listener);
  expect(listen).toHaveBeenCalledWith(events.importReceived, listener);
  unlisten();
  expect(stop).toHaveBeenCalledOnce();
});

it('loads the canonical generated F1 shapes and emits only subscribed fixture events', async () => {
  vi.mocked(isTauri).mockReturnValue(false);
  vi.stubEnv('VITE_XTRACE_FIXTURE', 'F1');
  const selected = await createDataSource();
  expect(selected.kind).toBe('fixture');
  expect(await selected.appInfo()).toEqual(exported.app_info);
  expect(await selected.dbCounts()).toEqual(exported.db_counts);
  const info = await selected.appInfo();
  info.version = 'changed';
  expect((await selected.appInfo()).version).toBe(exported.app_info.version);
  const fixture = await loadFixtureDataSource('F1');
  const imported = vi.fn();
  const stop = await fixture.subscribe(events.importReceived, imported);
  fixture.emit(events.prsRefreshed);
  expect(imported).not.toHaveBeenCalled();
  fixture.emit(events.importReceived);
  expect(imported).toHaveBeenCalledOnce();
  stop();
  fixture.emit(events.importReceived);
  expect(imported).toHaveBeenCalledOnce();
  expect(invoke).not.toHaveBeenCalled();
});

it('rejects skeleton and unrecognized fixtures without falling back to invented data', async () => {
  vi.mocked(isTauri).mockReturnValue(false);
  for (const id of ['F2', 'F20', '../F1']) {
    vi.stubEnv('VITE_XTRACE_FIXTURE', id);
    await expect(createDataSource()).rejects.toThrow('Only F1 is implemented');
  }
  expect(invoke).not.toHaveBeenCalled();
});

it('keeps ordinary and production browsers explicitly unavailable even with a production fixture flag', async () => {
  vi.mocked(isTauri).mockReturnValue(false);
  vi.stubEnv('VITE_XTRACE_FIXTURE', '');
  expect((await createDataSource()).kind).toBe('preview');
  vi.stubEnv('DEV', false);
  vi.stubEnv('VITE_XTRACE_FIXTURE', 'F1');
  const source = await createDataSource();
  expect(source.kind).toBe('preview');
  await expect(source.appInfo()).rejects.toThrow('Native data is unavailable');
  await expect(source.dbCounts()).rejects.toThrow('Native data is unavailable');
  expect(invoke).not.toHaveBeenCalled();
});
