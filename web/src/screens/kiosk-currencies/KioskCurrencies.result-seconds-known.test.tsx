// R4-1 (docs/design/web-silent-failures.md §8.1, §8.2): the queue machine's result_seconds field
// is "known" only after a settings read for the current branch succeeds. While it is not known the
// field is empty and Save, the change (blur / Enter) save and typing send no configureSettings.
// Up-front tests: rows (a)-(e) are red on main; row (f) is a guard and is green on main.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { ApiError, type ApiClient, type ViewParams } from '../../shared/api/client';
import {
  makeKbankReference,
  makeKioskCurrencies,
  makeRateFreshnessDto,
  makeViewAnswer,
} from '../../shared/testing/fixtures';
import { setLang } from '../../shared/i18n';
import { th as sharedTh } from '../../shared/i18n/th';
import { KioskCurrenciesScreen } from './KioskCurrenciesScreen';

type SettingsAnswer = { resultSeconds: number; version: number };

interface FakeClient {
  api: Pick<ApiClient, 'view' | 'post'>;
  posts: Array<{ path: string; body: unknown }>;
  /** Every configureSettings body the screen sent. */
  saves: () => Array<{ branchId: string; resultSeconds: unknown }>;
  /** Every settings read the screen sent, by branch. */
  reads: () => string[];
}

/**
 * The fake server. `settings(branchId)` answers each settings read; a configureSettings save
 * answers with the value it was sent.
 */
function createFakeClient(
  settings: (branchId: string) => Promise<SettingsAnswer>,
): FakeClient {
  const posts: FakeClient['posts'] = [];
  const api: Pick<ApiClient, 'view' | 'post'> = {
    view(_name: string, _q?: ViewParams) {
      const answer = makeViewAnswer(
        makeKioskCurrencies({
          currencies: [],
          kbank: makeKbankReference({ source: 'kbank', publishedAt: '2026-10-06T05:00:00Z', round: 1 }),
          freshness: makeRateFreshnessDto({ stale: false, sourceDate: null, publishedAt: null }),
        }),
        { version: 1, changed: true },
      );
      return Promise.resolve(answer as any);
    },
    post<B, R>(path: string, body: B): Promise<R> {
      posts.push({ path, body: JSON.parse(JSON.stringify(body)) as unknown });
      const b = body as { action: string; payload: { branchId: string; resultSeconds?: unknown } };
      if (b.action === 'settings') return settings(b.payload.branchId) as Promise<R>;
      if (b.action === 'configureSettings') {
        return Promise.resolve({
          resultSeconds: Number(b.payload.resultSeconds),
          version: 2,
        }) as Promise<R>;
      }
      return Promise.reject(new Error(`unexpected action ${b.action}`));
    },
  };
  const bodies = () =>
    posts.map((p) => p.body as { action: string; payload: { branchId: string; resultSeconds?: unknown } });
  return {
    api,
    posts,
    saves: () =>
      bodies()
        .filter((b) => b.action === 'configureSettings')
        .map((b) => ({ branchId: b.payload.branchId, resultSeconds: b.payload.resultSeconds })),
    reads: () => bodies().filter((b) => b.action === 'settings').map((b) => b.payload.branchId),
  };
}

function deferred<T>() {
  let resolve!: (v: T) => void;
  let reject!: (e: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

/** Lets every pending promise and Solid update run. */
async function settle(): Promise<void> {
  for (let i = 0; i < 5; i += 1) await new Promise((r) => setTimeout(r, 0));
}

describe('KioskCurrenciesScreen result_seconds is saved only once known (R4-1)', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;

  beforeEach(() => {
    setLang('th');
    host = document.createElement('div');
    document.body.appendChild(host);
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host.remove();
    document.body.innerHTML = '';
    setLang('th');
  });

  const getResultInput = () =>
    host.querySelector<HTMLInputElement>('input#result_seconds, input[name="result_seconds"]')!;
  const getSaveButton = () =>
    host.querySelector<HTMLButtonElement>('button[name="save_result_seconds"]')!;
  const getReadButton = () =>
    host.querySelector<HTMLButtonElement>('button[name="load_result_seconds"]')!;
  const getFieldError = () =>
    host.querySelector<HTMLElement>('[role="alert"], [data-error="validationFailed"]');

  /** Types `value` into the field (an input event), as a person does. */
  const type = (value: string) => {
    const input = getResultInput();
    input.value = value;
    input.dispatchEvent(new Event('input', { bubbles: true }));
  };
  /** Commits the field (the change event a blur or Enter fires). */
  const commit = () => {
    getResultInput().dispatchEvent(new Event('change', { bubbles: true }));
  };

  it('(a) on open the field is empty, not 10; Save and a change send no settings save', async () => {
    const fake = createFakeClient(() => new Promise<SettingsAnswer>(() => {}));
    dispose = render(() => <KioskCurrenciesScreen api={fake.api} branch="B9" />, host);
    await settle();

    const input = getResultInput();
    expect(input, 'the result_seconds field renders').not.toBeNull();
    expect(input.value, 'nothing has been read: the field is empty, never the default 10').toBe('');

    getSaveButton().click();
    await settle();
    expect(fake.saves(), 'Save before any read sends nothing').toEqual([]);

    type('15');
    commit();
    await settle();
    expect(fake.saves(), 'changing the field and leaving it before any read sends nothing').toEqual([]);
  });

  it('(b) focus reads the setting (7): Save sends nothing while the read is pending, then sends 7', async () => {
    const read = deferred<SettingsAnswer>();
    const fake = createFakeClient(() => read.promise);
    dispose = render(() => <KioskCurrenciesScreen api={fake.api} branch="B9" />, host);
    await settle();

    const input = getResultInput();
    input.focus();
    await vi.waitFor(() => expect(fake.reads()).toContain('B9'));

    getSaveButton().click();
    await settle();
    expect(fake.saves(), 'Save while the read is still pending sends nothing').toEqual([]);

    read.resolve({ resultSeconds: 7, version: 1 });
    await vi.waitFor(() => expect(input.value).toBe('7'));

    getSaveButton().click();
    await vi.waitFor(() =>
      expect(fake.saves()).toEqual([{ branchId: 'B9', resultSeconds: 7 }]),
    );
  });

  it('(c) a failed read shows the worded error, keeps Save disabled and sends nothing', async () => {
    const fake = createFakeClient(() =>
      Promise.reject(
        new ApiError({ status: 500, code: 'internal', message: 'raw upstream text' }),
      ),
    );
    dispose = render(() => <KioskCurrenciesScreen api={fake.api} branch="B9" />, host);
    await settle();

    getReadButton().click();
    await vi.waitFor(() => {
      const err = getFieldError();
      expect(err, 'the read failure is worded beside the field').not.toBeNull();
      expect(err!.textContent).toBe(sharedTh['error.serverError']);
    });

    expect(getResultInput().value, 'a failed read leaves the field empty').toBe('');
    expect(getSaveButton().disabled, 'Save stays disabled after a failed read').toBe(true);

    getSaveButton().click();
    commit();
    await settle();
    expect(fake.saves(), 'nothing is saved after a failed read').toEqual([]);
  });

  it('(d) after branch A reads 7, a change to branch B empties the field; nothing is saved until B is read', async () => {
    const [branch, setBranch] = createSignal('A');
    const fake = createFakeClient((b) =>
      Promise.resolve(b === 'A' ? { resultSeconds: 7, version: 1 } : { resultSeconds: 12, version: 3 }),
    );
    dispose = render(() => <KioskCurrenciesScreen api={fake.api} branch={branch()} />, host);
    await settle();

    getReadButton().click();
    await vi.waitFor(() => expect(getResultInput().value).toBe('7'));

    setBranch('B');
    await settle();
    expect(getResultInput().value, 'branch B has not been read: the field is empty, not A\'s 7').toBe('');

    getSaveButton().click();
    commit();
    await settle();
    expect(fake.saves(), 'nothing is saved over branch B before B is read').toEqual([]);

    getReadButton().click();
    await vi.waitFor(() => expect(getResultInput().value).toBe('12'));
    getSaveButton().click();
    await vi.waitFor(() =>
      expect(fake.saves()).toEqual([{ branchId: 'B', resultSeconds: 12 }]),
    );
  });

  it('(e) typing before any read is allowed, but Save and a change still send nothing', async () => {
    const fake = createFakeClient(() => new Promise<SettingsAnswer>(() => {}));
    dispose = render(() => <KioskCurrenciesScreen api={fake.api} branch="B9" />, host);
    await settle();

    type('25');
    expect(getResultInput().value, 'the typed text stays in the field').toBe('25');

    getSaveButton().click();
    commit();
    await settle();
    expect(fake.saves(), 'a typed value is never saved before a read succeeds').toEqual([]);
  });

  it('(f) guard: after a successful read, an edit and Save send the typed value', async () => {
    const fake = createFakeClient(() => Promise.resolve({ resultSeconds: 7, version: 1 }));
    dispose = render(() => <KioskCurrenciesScreen api={fake.api} branch="B9" />, host);
    await settle();

    getReadButton().click();
    await vi.waitFor(() => expect(getResultInput().value).toBe('7'));

    type('30');
    getSaveButton().click();
    await vi.waitFor(() =>
      expect(fake.saves()).toEqual([{ branchId: 'B9', resultSeconds: 30 }]),
    );
    expect(getResultInput().value).toBe('30');
  });
});
