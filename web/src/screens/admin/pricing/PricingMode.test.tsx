// V.9 `PricingMode` (PV-f4, #5199; docs/design/blueprint/pricing-tiers.md §15.1, §15.8a).
// Written before the implementation by claude/8be3a451. Do not edit, delete or weaken these tests.
//
// [required] by these tests (ticket #5199 behaviour table M1–M10):
// - `modeTitle` + `modeLabel(row.mode)`.
// - `row.tierSet` set: `tierSetSourceLabel(source)`, its 4 tiers' buy/sell, `tierSetApproved` with `approvedAt`
//   in Asia/Bangkok `dd/MM HH:mm`; null: `tierSetNone`.
// - `row.followResult` set: `followStatusLabel(status)`, one `followReasonLabel` per reason, `followPlannedAt` in
//   Bangkok time.
// - The switch only when `isMain && mayEdit`: mode FOLLOW ⇒ `button[name="mode-manual"]`, any other mode ⇒
//   `button[name="mode-follow"]`; pressing it sends one keyed `POST ROUTE_PRICING_MODE {currency, mode}`; 200 ⇒
//   `onSaved(out)`; an error ⇒ `wordedError`, no `onSaved`.
// - `!isMain && mayEdit` ⇒ no button, `modeMainOnly`; `!mayEdit` ⇒ neither.
import { afterEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { ApiClient } from '../../../shared/api/client';
import { setLang } from '../../../shared/i18n';
import { th as shellTh } from '../../../shared/i18n/th';
import {
  makePricingFollowResultDto,
  makePricingRowDto,
  makePricingTierRateDto,
  makePricingTierSetDto,
  type PricingRowDto,
} from '../../../shared/testing/fixtures/pricing_board';
import {
  makePricingModeSaveOutput,
  type PricingModeSaveOutput,
} from '../../../shared/testing/fixtures/pricing_mode_save';
import { boardFetch } from './__fixtures__/pricing-board.fixture';
import { th } from './i18n';
import { ROUTE_PRICING_MODE } from './pricing-labels';
import { PricingMode } from './PricingMode';

let host: HTMLDivElement;
let dispose: (() => void) | undefined;
afterEach(() => {
  dispose?.();
  dispose = undefined;
  host?.remove();
  setLang('th');
});

async function flush() {
  for (let i = 0; i < 8; i++) await Promise.resolve();
  await new Promise((r) => setTimeout(r, 0));
}

const SET_TIERS = [
  makePricingTierRateDto({ tierNo: 1, buy: '34.11', sell: '34.91' }),
  makePricingTierRateDto({ tierNo: 2, buy: '34.12', sell: '34.92' }),
  makePricingTierRateDto({ tierNo: 3, buy: '34.13', sell: '34.93' }),
  makePricingTierRateDto({ tierNo: 4, buy: '34.14', sell: '34.94' }),
];

const usd = (over: Partial<PricingRowDto> = {}) => makePricingRowDto({ currency: 'USD', mode: 'MANUAL', ...over });

function mount(
  row: PricingRowDto,
  opts: {
    isMain?: boolean;
    mayEdit?: boolean;
    onSaved?: (out: PricingModeSaveOutput) => void;
    command?: (path: string, body: unknown) => { status: number; body: unknown };
  } = {},
) {
  const onSaved = vi.fn(opts.onSaved ?? (() => {}));
  const savedOut = makePricingModeSaveOutput({ saved: true });
  const server = boardFetch(
    undefined,
    opts.command ?? ((path) => (path === ROUTE_PRICING_MODE ? { status: 200, body: savedOut } : { status: 404, body: {} })),
  );
  const api = new ApiClient(server.fn as unknown as typeof fetch);
  host = document.createElement('div');
  document.body.appendChild(host);
  dispose = render(
    () => (
      <PricingMode
        api={api}
        branch="MAIN"
        row={row}
        isMain={opts.isMain ?? true}
        mayEdit={opts.mayEdit ?? true}
        onSaved={onSaved}
      />
    ),
    host,
  );
  return { server, onSaved };
}

const modePosts = (s: ReturnType<typeof boardFetch>) =>
  s.calls.filter((c) => c.method === 'POST' && c.path === ROUTE_PRICING_MODE);
const switchButtons = () => host.querySelectorAll('button[name="mode-follow"], button[name="mode-manual"]');

describe('PricingMode', () => {
  it('M1 row mode MANUAL: "ตั้งเอง" shown', () => {
    mount(usd({ mode: 'MANUAL' }), { mayEdit: false });
    expect(host.textContent).toContain(th.modeTitle);
    expect(host.textContent).toContain('ตั้งเอง');
  });

  it('M2 tierSet MAIN_PUSH by exec1 at 2026-10-27T03:05:00Z: "จาก MAIN", the 4 tiers, approved by/at in Bangkok dd/MM HH:mm', () => {
    mount(
      usd({
        tierSet: makePricingTierSetDto({
          source: 'MAIN_PUSH',
          tiers: SET_TIERS,
          approvedBy: 'exec1',
          approvedAt: '2026-10-27T03:05:00Z',
        }),
      }),
      { mayEdit: false },
    );
    const text = host.textContent ?? '';
    expect(text).toContain(th.tierSetTitle);
    expect(text).toContain('จาก MAIN');
    for (const tier of SET_TIERS) {
      expect(text).toContain(tier.buy!);
      expect(text).toContain(tier.sell!);
    }
    expect(text).toContain('อนุมัติโดย exec1 เมื่อ 27/10 10:05');
    expect(text).not.toContain(th.tierSetNone);
  });

  it('M3 tierSet null: "ยังไม่มีราคาที่อนุมัติ"', () => {
    mount(usd({ tierSet: null }), { mayEdit: false });
    expect(host.textContent).toContain('ยังไม่มีราคาที่อนุมัติ');
  });

  it('M4 FOLLOW with followResult BLOCKED (STALE_SOURCE, LARGE_MOVE): status, both reasons and the planned time', () => {
    mount(
      usd({
        mode: 'FOLLOW',
        followResult: makePricingFollowResultDto({
          currency: 'USD',
          status: 'BLOCKED',
          reasons: ['STALE_SOURCE', 'LARGE_MOVE'],
          plannedAt: '2026-10-27T04:20:00Z',
        }),
      }),
      { mayEdit: false },
    );
    const text = host.textContent ?? '';
    expect(text).toContain(th.followTitle);
    expect(text).toContain('ถูกบล็อก');
    expect(text).toContain('ราคาต้นทางเก่า');
    expect(text).toContain('ราคาขยับมากเกินไป');
    // followPlannedAt with plannedAt in Bangkok time (04:20Z = 11:20 ICT).
    expect(text).toContain('คำนวณล่าสุด');
    expect(text).toContain('11:20');
    expect(text).not.toContain('04:20');
  });

  it('M5 isMain, mayEdit, mode MANUAL, press mode-follow: one keyed POST /api/pricing/mode {currency, mode: FOLLOW}; onSaved called', async () => {
    const { server, onSaved } = mount(usd({ mode: 'MANUAL' }), { isMain: true, mayEdit: true });
    const b = host.querySelector<HTMLButtonElement>('button[name="mode-follow"]');
    expect(b, 'button[name="mode-follow"] exists').not.toBeNull();
    expect(b!.textContent).toContain(th.switchToFollow);
    b!.click();
    await flush();
    const sent = modePosts(server);
    expect(sent).toHaveLength(1);
    expect(sent[0].path).toBe('/api/pricing/mode');
    expect(sent[0].body).toEqual({ currency: 'USD', mode: 'FOLLOW' });
    expect(sent[0].key, 'Idempotency-Key header').toBeTruthy();
    expect(onSaved).toHaveBeenCalledTimes(1);
    expect(onSaved).toHaveBeenCalledWith(makePricingModeSaveOutput({ saved: true }));
  });

  it('M6 isMain, mayEdit, mode FOLLOW, press mode-manual: the body mode is "MANUAL"', async () => {
    const { server } = mount(usd({ mode: 'FOLLOW' }), { isMain: true, mayEdit: true });
    expect(host.querySelector('button[name="mode-follow"]')).toBeNull();
    const b = host.querySelector<HTMLButtonElement>('button[name="mode-manual"]');
    expect(b, 'button[name="mode-manual"] exists').not.toBeNull();
    expect(b!.textContent).toContain(th.switchToManual);
    b!.click();
    await flush();
    const sent = modePosts(server);
    expect(sent).toHaveLength(1);
    expect((sent[0].body as Record<string, unknown>).currency).toBe('USD');
    expect((sent[0].body as Record<string, unknown>).mode).toBe('MANUAL');
  });

  it('M7 isMain, mayEdit, mode AUTO: the mode-follow button is shown', () => {
    mount(usd({ mode: 'AUTO' }), { isMain: true, mayEdit: true });
    expect(host.querySelector('button[name="mode-follow"]')).not.toBeNull();
    expect(host.querySelector('button[name="mode-manual"]')).toBeNull();
  });

  it('M8 not MAIN, mayEdit: no switch button; "เปลี่ยนโหมดได้ที่สาขา MAIN เท่านั้น" shown', () => {
    mount(usd({ mode: 'MANUAL' }), { isMain: false, mayEdit: true });
    expect(switchButtons()).toHaveLength(0);
    expect(host.textContent).toContain('เปลี่ยนโหมดได้ที่สาขา MAIN เท่านั้น');
  });

  it('M9 mayEdit false: no switch button and no main-only text (at MAIN or elsewhere)', () => {
    for (const isMain of [true, false]) {
      mount(usd({ mode: 'MANUAL' }), { isMain, mayEdit: false });
      // the panel itself still renders
      expect(host.textContent).toContain(th.modeTitle);
      expect(switchButtons()).toHaveLength(0);
      expect(host.textContent).not.toContain(th.modeMainOnly);
      dispose?.();
      dispose = undefined;
      host.remove();
    }
  });

  it('M10 the POST answers 403 roleNotAllowedHere: worded error shown; onSaved not called', async () => {
    const { server, onSaved } = mount(usd({ mode: 'MANUAL' }), {
      isMain: true,
      mayEdit: true,
      command: () => ({ status: 403, body: { error: { code: 'roleNotAllowedHere', message: 'RAW wire text' } } }),
    });
    const b = host.querySelector<HTMLButtonElement>('button[name="mode-follow"]');
    expect(b, 'button[name="mode-follow"] exists').not.toBeNull();
    b!.click();
    await flush();
    expect(modePosts(server)).toHaveLength(1);
    expect(host.textContent).toContain(shellTh['error.roleNotAllowedHere']);
    expect(host.textContent).not.toContain('RAW wire text');
    expect(onSaved).not.toHaveBeenCalled();
  });
});
