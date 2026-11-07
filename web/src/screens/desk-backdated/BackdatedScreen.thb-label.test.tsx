// OL-7 row 14 (VERIFY.md on main 4a577dac, shot 007): the /desk/backdated summary reads "ยอดคำนวณ (THB): — THB", the
// currency twice: the label carries "(THB)" and the value (BackdatedScreen.tsx `{computedThb() ?? '—'} THB`) carries it
// again, and so does the net-settle line. The value keeps its unit (BackdatedScreen.units.test.tsx pins "2155.50 THB");
// the two summary labels drop theirs. Input labels such as "ค่าธรรมเนียม (THB)" have no unit after the box and keep it.
// Written before the fix by claude/studio-claude-17. Do not edit, delete or weaken.
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { render } from 'solid-js/web';
import { ApiClient } from '../../shared/api/client';
import { setLang } from '../../shared/i18n';
import { BackdatedScreen } from './BackdatedScreen';
import { en, th } from './i18n';

let host: HTMLDivElement;
let dispose: (() => void) | undefined;

function mount(): void {
  host = document.createElement('div');
  document.body.appendChild(host);
  const api = new ApiClient(
    async () =>
      new Response(JSON.stringify({ version: 1, changed: true, data: { currencies: [], rates: [] } }), {
        status: 200,
        headers: { 'content-type': 'application/json' },
      }),
  );
  dispose = render(() => <BackdatedScreen api={api} me={() => undefined} />, host);
}

const line = () => (host.querySelector('[data-testid="computed-thb"]')?.textContent ?? '').replace(/\s+/g, ' ').trim();
const count = (s: string, word: string) => s.split(word).length - 1;

beforeEach(() => setLang('th'));

afterEach(() => {
  dispose?.();
  dispose = undefined;
  host?.remove();
  setLang('th');
});

describe('backdated summary names THB once', () => {
  it('BT-1 Thai: the empty computed line reads "ยอดคำนวณ: — THB"', () => {
    mount();
    expect(line()).toMatch(/^ยอดคำนวณ:\s*— THB$/);
    expect(count(line(), 'THB')).toBe(1);
  });

  it('BT-2 English: the empty computed line reads "Computed: — THB"', () => {
    setLang('en');
    mount();
    expect(line()).toMatch(/^Computed:\s*— THB$/);
    expect(count(line(), 'THB')).toBe(1);
  });

  it('BT-3 the two summary labels carry no unit; the value after them does', () => {
    expect(th.computedThb).toBe('ยอดคำนวณ');
    expect(th.settleThb).toBe('ยอดสุทธิ');
    expect(en.computedThb).toBe('Computed');
    expect(en.settleThb).toBe('Net settle');
  });

  it('BT-4 guard: input labels keep "(THB)", since no unit follows the box', () => {
    expect(th.feeThb).toBe('ค่าธรรมเนียม (THB)');
    expect(en.feeThb).toBe('Fee (THB)');
  });
});
