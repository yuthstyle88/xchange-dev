// Ticket #2612: AlertSettings branch switch while a save is in flight.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { AlertSettings } from './AlertSettings';
import * as table from './i18n';

const BRANCH_A = 'XC-B01';
const BRANCH_B = 'XC-B02';
const VIEW = '/api/v3/views/branch_alert_settings';
const SAVE = '/api/v3/commands/branch_alert_settings_save';

interface CapturedSave {
  branch: string;
  body: Record<string, unknown>;
  resolve: (res: Response) => void;
}

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

describe('#2612 AlertSettings: save answering after a branch switch', () => {
  let root: HTMLDivElement;
  let dispose: (() => void) | undefined;
  let viewCalls: string[];
  let viewHandlers: Record<string, () => Promise<Response>>;
  let pendingSaves: CapturedSave[];

  const initialA = {
    branch: BRANCH_A,
    staleDataMinutes: 20,
    floors: [{ currency: 'USD', minQuantity: '1000' }],
    version: 3,
  };

  const initialB = {
    branch: BRANCH_B,
    staleDataMinutes: 35,
    floors: [{ currency: 'EUR', minQuantity: '500' }],
    version: 7,
  };

  beforeEach(() => {
    setLang('th');
    viewCalls = [];
    pendingSaves = [];
    viewHandlers = {
      [BRANCH_A]: async () =>
        json(200, { version: 3, changed: true, data: structuredClone(initialA) }),
      [BRANCH_B]: async () =>
        json(200, { version: 7, changed: true, data: structuredClone(initialB) }),
    };

    root = document.createElement('div');
    document.body.appendChild(root);

    vi.stubGlobal('fetch', async (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
      const raw = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const u = new URL(raw, 'http://localhost');
      const method = (init?.method ?? 'GET').toUpperCase();

      if (method === 'GET' && u.pathname === VIEW) {
        const b = u.searchParams.get('branch') ?? '';
        viewCalls.push(b);
        const handler = viewHandlers[b];
        if (handler) return handler();
        return json(200, { version: 1, changed: true, data: null });
      }

      if (method === 'POST' && u.pathname === SAVE) {
        const rawBody = typeof init?.body === 'string' ? init.body : '{}';
        const parsed = JSON.parse(rawBody) as Record<string, unknown>;
        return new Promise<Response>((resolve) => {
          pendingSaves.push({
            branch: String(parsed.branch ?? ''),
            body: parsed,
            resolve,
          });
        });
      }

      return json(200, { version: 1, changed: false });
    });
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    root.remove();
    vi.restoreAllMocks();
  });

  async function settle(): Promise<void> {
    for (let i = 0; i < 8; i++) {
      await new Promise((r) => setTimeout(r, 0));
    }
  }

  function mountScreen(initialBranch = BRANCH_A) {
    const [branch, setBranch] = createSignal(initialBranch);
    const [period] = createSignal({ from: '2026-10-16', to: '2026-10-16' });
    const [shown] = createSignal(true);
    dispose = render(
      () => <AlertSettings branch={branch} period={period} shown={shown} />,
      root,
    );
    return { branch, setBranch };
  }

  function minutesInput(): HTMLInputElement {
    const input = root.querySelector('#stale-data-minutes') as HTMLInputElement | null;
    expect(input, 'staleDataMinutes input exists').not.toBeNull();
    return input!;
  }

  function floorInput(currency: string): HTMLInputElement | null {
    return root.querySelector(`[data-floor="${currency}"] input`) as HTMLInputElement | null;
  }

  function floorCurrencies(): string[] {
    return Array.from(root.querySelectorAll('[data-floor]')).map(
      (el) => el.getAttribute('data-floor') ?? '',
    );
  }

  function saveButton(): HTMLButtonElement {
    const btn = Array.from(root.querySelectorAll('button')).find(
      (b) => b.textContent?.trim() === table.th.save,
    );
    expect(btn, 'save button exists').toBeDefined();
    return btn!;
  }

  function alertBanner(): HTMLElement | null {
    return root.querySelector('[role="alert"]');
  }

  function setInputValue(input: HTMLInputElement, value: string): void {
    input.value = value;
    input.dispatchEvent(new Event('input', { bubbles: true }));
    input.dispatchEvent(new Event('change', { bubbles: true }));
  }

  it("row 1: Save on A answers after a switch to B -> B's form shows B's own floors and version; the next Save on B carries B's expectedVersion", async () => {
    const { setBranch } = mountScreen(BRANCH_A);
    await settle();

    expect(minutesInput().value).toBe('20');
    expect(floorCurrencies()).toEqual(['USD']);

    // Start Save on A
    saveButton().click();
    await settle();
    expect(pendingSaves).toHaveLength(1);
    expect(pendingSaves[0]!.branch).toBe(BRANCH_A);

    // Switch to B while Save on A is in flight
    setBranch(BRANCH_B);
    await settle();

    expect(minutesInput().value).toBe('35');
    expect(floorCurrencies()).toEqual(['EUR']);
    expect(floorInput('EUR')?.value).toBe('500');

    // Now Save on A answers with A's updated floors and version 4
    pendingSaves[0]!.resolve(
      json(200, {
        branch: BRANCH_A,
        staleDataMinutes: 45,
        floors: [{ currency: 'USD', minQuantity: '2000' }],
        version: 4,
      }),
    );
    await settle();

    // B's form still shows B's own floors and minutes
    expect(minutesInput().value).toBe('35');
    expect(floorCurrencies()).toEqual(['EUR']);
    expect(floorInput('EUR')?.value).toBe('500');
    expect(floorInput('USD')).toBeNull();

    // Next Save on B carries B's expectedVersion (7), not A's (4)
    saveButton().click();
    await settle();
    expect(pendingSaves).toHaveLength(2);
    expect(pendingSaves[1]!.branch).toBe(BRANCH_B);
    expect(pendingSaves[1]!.body.expectedVersion).toBe(7);
    expect(pendingSaves[1]!.body.floors).toEqual([{ currency: 'EUR', minQuantity: '500' }]);
  });

  it('row 2: Save on A refused after a switch to B -> No error shown on B', async () => {
    const { setBranch } = mountScreen(BRANCH_A);
    await settle();

    // Start Save on A
    saveButton().click();
    await settle();
    expect(pendingSaves).toHaveLength(1);

    // Switch to B while Save on A is in flight
    setBranch(BRANCH_B);
    await settle();

    // Save on A is refused with 400 validationFailed
    pendingSaves[0]!.resolve(
      json(400, {
        error: { code: 'validationFailed', message: 'A validation refusal' },
      }),
    );
    await settle();

    expect(alertBanner()).toBeNull();
    expect(minutesInput().value).toBe('35');
    expect(floorCurrencies()).toEqual(['EUR']);
  });

  it("row 3: staleVersion on A after a switch to B -> No branch_alert_settings read for A after the switch; B's data stays", async () => {
    const { setBranch } = mountScreen(BRANCH_A);
    await settle();
    expect(viewCalls).toEqual([BRANCH_A]);

    // Start Save on A
    saveButton().click();
    await settle();
    expect(pendingSaves).toHaveLength(1);

    // Switch to B
    setBranch(BRANCH_B);
    await settle();
    expect(viewCalls).toEqual([BRANCH_A, BRANCH_B]);

    // Save on A answers 409 staleVersion after switch to B
    pendingSaves[0]!.resolve(
      json(409, {
        error: { code: 'staleVersion', message: 'stale version on A' },
      }),
    );
    await settle();

    // No extra read for A after the switch; B's data and no error stay
    expect(viewCalls).toEqual([BRANCH_A, BRANCH_B]);
    expect(alertBanner()).toBeNull();
    expect(minutesInput().value).toBe('35');
    expect(floorCurrencies()).toEqual(['EUR']);
    expect(floorInput('EUR')?.value).toBe('500');
  });

  it('row 4: Save with no switch -> Form takes the answered set and version (kept)', async () => {
    const { setBranch } = mountScreen(BRANCH_A);
    await settle();

    // First trigger an in-flight save on A and switch to B so late A answer is ignored on B
    saveButton().click();
    await settle();
    setBranch(BRANCH_B);
    await settle();
    pendingSaves[0]!.resolve(
      json(200, {
        branch: BRANCH_A,
        staleDataMinutes: 99,
        floors: [{ currency: 'USD', minQuantity: '9999' }],
        version: 4,
      }),
    );
    await settle();

    // Now on B, perform a Save with no switch: form takes the answered set and version
    setInputValue(minutesInput(), '40');
    setInputValue(floorInput('EUR')!, '600');
    saveButton().click();
    await settle();

    expect(pendingSaves).toHaveLength(2);
    expect(pendingSaves[1]!.branch).toBe(BRANCH_B);
    expect(pendingSaves[1]!.body.expectedVersion).toBe(7);

    pendingSaves[1]!.resolve(
      json(200, {
        branch: BRANCH_B,
        staleDataMinutes: 40,
        floors: [{ currency: 'EUR', minQuantity: '600' }],
        version: 8,
      }),
    );
    await settle();

    expect(minutesInput().value).toBe('40');
    expect(floorCurrencies()).toEqual(['EUR']);
    expect(floorInput('EUR')?.value).toBe('600');

    // Verify version 8 was taken by triggering one more save on B
    saveButton().click();
    await settle();
    expect(pendingSaves).toHaveLength(3);
    expect(pendingSaves[2]!.body.expectedVersion).toBe(8);
  });

  it("edge case (§9): Switching back to A after the save answered shows A's data as read fresh", async () => {
    const { setBranch } = mountScreen(BRANCH_A);
    await settle();

    // Start Save on A, switch to B, then switch back to A before A's first save answers
    saveButton().click();
    await settle();
    expect(pendingSaves).toHaveLength(1);

    setBranch(BRANCH_B);
    await settle();

    // Update what A's fresh read returns when switching back to A
    let resolveFreshA!: (res: Response) => void;
    viewHandlers[BRANCH_A] = () =>
      new Promise<Response>((r) => {
        resolveFreshA = r;
      });

    setBranch(BRANCH_A);
    await settle();

    // Old save from the first visit to A answers while A's fresh read is in flight
    pendingSaves[0]!.resolve(
      json(200, {
        branch: BRANCH_A,
        staleDataMinutes: 25,
        floors: [{ currency: 'USD', minQuantity: '1200' }],
        version: 4,
      }),
    );
    await settle();

    // Fresh read on A answers with the latest server state (version 5)
    resolveFreshA(
      json(200, {
        version: 5,
        changed: true,
        data: {
          branch: BRANCH_A,
          staleDataMinutes: 50,
          floors: [{ currency: 'JPY', minQuantity: '80000' }],
          version: 5,
        },
      }),
    );
    await settle();

    expect(minutesInput().value).toBe('50');
    expect(floorCurrencies()).toEqual(['JPY']);
    expect(floorInput('JPY')?.value).toBe('80000');

    // Also if another save on A was in flight and answered after the fresh read after A -> B -> A, fresh data stays
    saveButton().click();
    await settle();
    expect(pendingSaves).toHaveLength(2);
    expect(pendingSaves[1]!.body.expectedVersion).toBe(5);

    setBranch(BRANCH_B);
    await settle();
    viewHandlers[BRANCH_A] = async () =>
      json(200, {
        version: 6,
        changed: true,
        data: {
          branch: BRANCH_A,
          staleDataMinutes: 60,
          floors: [{ currency: 'GBP', minQuantity: '300' }],
          version: 6,
        },
      });
    setBranch(BRANCH_A);
    await settle();

    // Now the in-flight save from before A -> B -> A answers with version 5 data
    pendingSaves[1]!.resolve(
      json(200, {
        branch: BRANCH_A,
        staleDataMinutes: 50,
        floors: [{ currency: 'JPY', minQuantity: '80000' }],
        version: 5,
      }),
    );
    await settle();

    // Freshly read version 6 data on A stays intact
    expect(minutesInput().value).toBe('60');
    expect(floorCurrencies()).toEqual(['GBP']);
    expect(floorInput('GBP')?.value).toBe('300');
  });
});
