// #6286 T7: the "Customer app" admin screen while the OTP channels are not set.
// Written before the implementation by claude/studio-claude-17 on the lane controller's order. Do not edit,
// delete or weaken; add tests in a file of your own. The API side is `api/tests/otp_channels_store_test.rs`.
//
// What this test assumes (the implementer follows them):
// - `./menu.ts` exports `menu`, id `customer-app`, `needs: 'SystemSettings'`, `Component: CustomerApp`. It is read
//   through an eager glob, so without it the test fails on an assertion, not on the import.
// - The screen reads `GET /api/auth/otp/channels`, which answers `{ channels, source }`.
// - DOM hooks: `[data-otp-source="default"]` holds the text "ค่าเริ่มต้น" while not set; one checkbox per channel,
//   `input[type="checkbox"][data-otp-channel="<channel>"]`, in the order they apply.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import type { AdminMenu } from '../menus';
import { CustomerApp } from './CustomerApp';

const menuFiles = import.meta.glob<{ menu: AdminMenu }>('./menu.ts', { eager: true });

let dispose: (() => void) | undefined;
let root: HTMLElement | undefined;

async function settle(): Promise<void> {
  for (let round = 0; round < 4; round++) {
    for (let i = 0; i < 5; i++) await Promise.resolve();
    await new Promise((r) => setTimeout(r, 0));
  }
}

function mount(): HTMLElement {
  const [branch] = createSignal('XC-B01');
  const [period] = createSignal({ from: '2026-11-06', to: '2026-11-06' });
  const [shown] = createSignal(true);
  const el = document.createElement('div');
  root = el;
  document.body.appendChild(el);
  dispose = render(
    () => (<CustomerApp branch={branch} period={period} shown={shown} />) as JSX.Element,
    el,
  );
  return el;
}

beforeEach(() => {
  setLang('th');
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: string) => {
      const path = new URL(input, 'http://localhost').pathname;
      const body =
        path === '/api/auth/otp/channels'
          ? { channels: ['email', 'sms'], source: 'default' }
          : { error: { code: 'notFound', message: path } };
      return new Response(JSON.stringify(body), {
        status: path === '/api/auth/otp/channels' ? 200 : 404,
        headers: { 'content-type': 'application/json' },
      });
    }),
  );
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  root?.remove();
  root = undefined;
  vi.unstubAllGlobals();
});

describe('CustomerApp (#6286)', () => {
  it('T7: not set shows ค่าเริ่มต้น and both boxes ticked, email then sms', async () => {
    const el = mount();
    await settle();

    expect(el.querySelector('[data-otp-source="default"]')?.textContent).toContain('ค่าเริ่มต้น');
    const boxes = [...el.querySelectorAll<HTMLInputElement>('input[type="checkbox"][data-otp-channel]')];
    expect(boxes.map((b) => [b.dataset.otpChannel, b.checked])).toEqual([
      ['email', true],
      ['sms', true],
    ]);
  });

  it('is the admin menu customer-app, for those holding SystemSettings', () => {
    const m = menuFiles['./menu.ts'];
    expect(m, 'web/src/screens/admin/customer-app/menu.ts exists').toBeDefined();
    expect(m!.menu.id).toBe('customer-app');
    expect(m!.menu.needs).toBe('SystemSettings');
    expect(m!.menu.Component).toBe(CustomerApp);
  });
});
