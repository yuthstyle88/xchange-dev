import { describe, expect, it, vi } from 'vitest';
import type { ApiClient } from '../../shared/api/client';
import type { DeskEntryState, DeskStore } from './desk-store';
import { collectEntries, openEntry, type DeskEntry } from './entries';

function makeEntry(load: DeskEntry['load'] = vi.fn()): { entry: DeskEntry } {
  return {
    entry: {
      param: 'kiosk',
      load,
    },
  };
}

describe('entries', () => {
  it('row 1: throws "Invalid entry path" on a bad module path', () => {
    expect(() => collectEntries({ './entries/Kiosk/entry.ts': makeEntry() })).toThrow('Invalid entry path');
    expect(() => collectEntries({ './entries/kiosk/index.ts': makeEntry() })).toThrow('Invalid entry path');
  });

  it('row 2: throws "Missing entry export" when a module has no entry export', () => {
    expect(() => collectEntries({ './entries/kiosk/entry.ts': {} })).toThrow('Missing entry export');
  });

  it('row 3: throws "Duplicate…" when two modules have the same param', () => {
    const glob = {
      './entries/kiosk/entry.ts': makeEntry(),
      './entries/other/entry.ts': makeEntry(),
    };
    expect(() => collectEntries(glob)).toThrow(/Duplicate/);
  });

  it('row 4: does not call load or setEntry when ?kiosk= is empty', async () => {
    const load = vi.fn();
    const setEntry = vi.fn();
    const store = { api: {} as ApiClient, setEntry } as unknown as DeskStore;
    const entries = collectEntries({ './entries/kiosk/entry.ts': makeEntry(load) });

    await openEntry(store, entries, '?kiosk=');

    expect(load).not.toHaveBeenCalled();
    expect(setEntry).not.toHaveBeenCalled();
  });

  it('row 5: calls load(store, "K1", store.api) once and then setEntry(result) when ?kiosk=K1', async () => {
    const result = { kind: 'kiosk', id: 'K1' } as unknown as DeskEntryState;
    const load = vi.fn(async () => result);
    const setEntry = vi.fn();
    const api = { command: vi.fn(), view: vi.fn() } as unknown as ApiClient;
    const store = { api, setEntry } as unknown as DeskStore;
    const entries = collectEntries({ './entries/kiosk/entry.ts': makeEntry(load) });

    await openEntry(store, entries, '?kiosk=K1');

    expect(load).toHaveBeenCalledTimes(1);
    expect(load).toHaveBeenCalledWith(store, 'K1', api);
    expect(setEntry).toHaveBeenCalledTimes(1);
    expect(setEntry).toHaveBeenCalledWith(result);
  });
});
