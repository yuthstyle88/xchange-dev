// The desk's entries (design xchange-v3-wave3-desk.md §5.3, §6): `?kiosk=` opens the desk on a kiosk
// request. Each lives in ./entries/<name>/entry.ts (DK-I) and is found through the glob, so an entry ticket
// adds files only in its own folder.
import type { ApiClient } from '../../shared/api/client';
import type { DeskEntryState, DeskStore } from './desk-store';

export interface DeskEntry {
  param: 'booking' | 'kiosk';
  load(store: DeskStore, id: string, api: ApiClient): Promise<DeskEntryState>;
}

const ENTRY_PATH_RE = /^\.\/entries\/[a-z][a-z0-9-]*\/entry\.ts$/;

/** The entries of `import.meta.glob('./entries/*\/entry.ts', { eager: true })`, one per URL parameter. */
export function collectEntries(glob: Record<string, unknown>): DeskEntry[] {
  const byParam = new Map<DeskEntry['param'], string>();
  const entries: DeskEntry[] = [];
  for (const [path, mod] of Object.entries(glob)) {
    if (!ENTRY_PATH_RE.test(path)) {
      throw new Error(`Invalid entry path: "${path}"`);
    }
    const m = mod as { entry?: DeskEntry } | null | undefined;
    if (!m || typeof m !== 'object' || !m.entry) {
      throw new Error(`Missing entry export in "${path}"`);
    }
    const seen = byParam.get(m.entry.param);
    if (seen) {
      throw new Error(`Duplicate entry param "${m.entry.param}" in "${seen}" and "${path}"`);
    }
    byParam.set(m.entry.param, path);
    entries.push(m.entry);
  }
  return entries;
}

const ENTRY_MODULES = import.meta.glob('./entries/*/entry.ts', { eager: true });

/** Opens the entry the page URL names (`?kiosk=`), if any, and hands its state to the store. */
export async function openEntry(
  store: DeskStore,
  entries: DeskEntry[] = collectEntries(ENTRY_MODULES),
  search: string = typeof location !== 'undefined' ? location.search : '',
): Promise<void> {
  const params = new URLSearchParams(search);
  for (const e of entries) {
    const id = params.get(e.param);
    if (id) {
      store.setEntry(await e.load(store, id, store.api));
      return;
    }
  }
}
