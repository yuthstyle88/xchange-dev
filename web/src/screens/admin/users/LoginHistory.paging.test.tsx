// OL-6 B admin-users: 100 login rows were one 6000px table (shot 045). The view is read once (limit 100); the table
// shows 25 at a time. Raw user ids in the users and audit tables are shortened only where they are ids.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { LoginHistory } from './LoginHistory';
import { shortId } from './short-id';

const disposers: Array<() => void> = [];
let gets = 0;
let rowCount = 100;

const rowsOf = (n: number) =>
  Array.from({ length: n }, (_, i) => ({
    id: `l${i}`,
    at: '2026-11-07T01:00:00Z',
    userId: `u${i}`,
    userName: `User ${i}`,
    ip: null,
    result: 'OK',
  }));

async function flush(): Promise<void> {
  for (let i = 0; i < 10; i += 1) await new Promise((r) => setTimeout(r, 0));
}

function mount(branch: () => string = () => 'XC-B01'): HTMLElement {
  const c = document.createElement('div');
  document.body.appendChild(c);
  const dispose = render(() => <LoginHistory shown={() => true} branch={branch} />, c);
  disposers.push(() => {
    dispose();
    c.remove();
  });
  return c;
}

const names = (c: HTMLElement) => [...c.querySelectorAll('tbody tr')].map((r) => r.children[1]!.textContent);
const pagerButtons = (c: HTMLElement) => [...c.querySelectorAll<HTMLButtonElement>('[data-logins-pager] button')];

describe('login history paging', () => {
  beforeEach(() => {
    setLang('th');
    gets = 0;
    rowCount = 100;
    vi.stubGlobal('fetch', async () => {
      gets += 1;
      return new Response(JSON.stringify(rowsOf(rowCount)), { status: 200, headers: { 'content-type': 'application/json' } });
    });
  });

  afterEach(() => {
    for (const d of disposers.splice(0)) d();
    vi.unstubAllGlobals();
  });

  it('LH-1 100 rows: first 25 shown in server order, page 1 / 4, Previous disabled', async () => {
    const c = mount();
    await flush();
    expect(names(c)).toEqual(rowsOf(25).map((r) => r.userName));
    expect(c.querySelector('[data-logins-page]')?.textContent).toBe('หน้า 1 / 4');
    expect(pagerButtons(c).map((b) => b.disabled)).toEqual([true, false]);
  });

  it('LH-2 Next shows the next 25 with no new request; the last page disables Next', async () => {
    const c = mount();
    await flush();
    pagerButtons(c)[1]!.click();
    await flush();
    expect(names(c)[0]).toBe('User 25');
    expect(names(c)).toHaveLength(25);
    expect(gets).toBe(1);
    pagerButtons(c)[1]!.click();
    pagerButtons(c)[1]!.click();
    await flush();
    expect(names(c)[0]).toBe('User 75');
    expect(c.querySelector('[data-logins-page]')?.textContent).toBe('หน้า 4 / 4');
    expect(pagerButtons(c).map((b) => b.disabled)).toEqual([false, true]);
    pagerButtons(c)[0]!.click();
    await flush();
    expect(names(c)[0]).toBe('User 50');
  });

  it('LH-3 25 rows or fewer: no pager', async () => {
    rowCount = 25;
    const c = mount();
    await flush();
    expect(names(c)).toHaveLength(25);
    expect(c.querySelector('[data-logins-pager]')).toBeNull();
  });

  it('LH-4 26 rows: two pages, the second holds one row', async () => {
    rowCount = 26;
    const c = mount();
    await flush();
    pagerButtons(c)[1]!.click();
    await flush();
    expect(names(c)).toEqual(['User 25']);
  });

  it('LH-5 a reload (branch change) goes back to page 1', async () => {
    const [branch, setBranch] = createSignal('XC-B01');
    const c = mount(branch);
    await flush();
    pagerButtons(c)[1]!.click();
    await flush();
    setBranch('XC-B02');
    await flush();
    expect(names(c)[0]).toBe('User 0');
    expect(c.querySelector('[data-logins-page]')?.textContent).toBe('หน้า 1 / 4');
  });
});

describe('shortId', () => {
  it('SI-1 a UUID shows its first block', () => {
    expect(shortId('5fe9d9ce-54a4-4ffd-b765-68b963271000')).toBe('5fe9d9ce');
  });

  it('SI-2 anything else is left alone', () => {
    expect(shortId('u-1')).toBe('u-1');
    expect(shortId('')).toBe('');
  });
});
