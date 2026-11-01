// FE-T1a #5851 (design docs/design/fe-triage-1101.md §3 W2, W3; triage T1, T7(a) on #564). Written before the
// implementation by the FE sub-controller claude/01TxaRJG. Do not edit, delete or weaken; add tests in a file of your own.
//
// [required] by these tests:
// - W2: a click on `button[name=finish]` posts nothing and reveals `button[name=finishConfirm]`, reading
//   "ยืนยันเสร็จสิ้น" (th) / "Confirm finish" (en) from `./i18n.ts` key `finishConfirm`. The confirm sends exactly one
//   `commands.ticket('finish', id)`, then `onDone()` once; a second click while it is in flight sends nothing.
//   Same two-step shape as release (`TicketActions.release.test.tsx` R2, R3).
// - W3: a `409 alreadyServing` refusal shows `./i18n.ts` key `error.alreadyServing`, never the shared
//   `error.serverError` wording.
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { ApiError } from '../../../../shared/api/client';
import { setLang } from '../../../../shared/i18n';
import { th as sharedTh } from '../../../../shared/i18n/th';
import { en as sharedEn } from '../../../../shared/i18n/en';
import { TicketActions } from './TicketActions';
import { en, th } from './i18n';

const ME = { userId: 'u-me', permissions: ['QueueOperate'] };
const MANAGER = { userId: 'u-boss', permissions: ['QueueOperate', 'BranchDayOperate'] };
const ticket = (status: string, tellerId: string | null) => ({ id: 't-1', displayNumber: 'B004', status, tellerId });

const W3_TH = 'เคาน์เตอร์นี้กำลังให้บริการคิวอื่นอยู่ กรุณาเสร็จสิ้นคิวปัจจุบันก่อน';
const W3_EN = 'This counter is already serving a ticket. Finish it first.';

function fakeCommands() {
  const answer = { id: 't-1', displayNumber: 'B004', serviceType: 'BOOKING', sequence: 4, status: 'COMPLETED', tellerId: 'u-me', endedReason: null };
  return {
    issueWalkIn: vi.fn(async () => answer),
    callNext: vi.fn(async () => answer),
    ticket: vi.fn(async (_verb: string, _id: string) => answer),
    cancel: vi.fn(async (_id: string, _reason: string) => answer),
    counter: vi.fn(async () => ({ tellerId: 'u-me', status: 'ACTIVE' })),
    queue: vi.fn(async () => ({ queueOpen: true, activeCounters: 1 })),
  };
}

let host: HTMLDivElement;
let dispose: (() => void) | undefined;
afterEach(() => {
  dispose?.();
  dispose = undefined;
  host?.remove();
  setLang('th');
});

function mount(el: () => JSX.Element) {
  host = document.createElement('div');
  document.body.appendChild(host);
  dispose = render(el, host);
}
const button = (n: string) => host.querySelector<HTMLButtonElement>(`button[name=${n}]`);
async function flush() {
  for (let i = 0; i < 6; i++) await Promise.resolve();
}

describe('TicketActions: finish needs a confirm (FE-T1a W2)', () => {
  it('T1a-0 the confirm text is th ยืนยันเสร็จสิ้น / en Confirm finish', () => {
    expect((th as Record<string, string>).finishConfirm).toBe('ยืนยันเสร็จสิ้น');
    expect((en as Record<string, string>).finishConfirm).toBe('Confirm finish');
  });

  it.each([
    ['the serving teller', ME, 'u-me'],
    ['a branch manager', MANAGER, 'u-other'],
  ] as const)('T1a-1 SERVING, %s: a click on finish posts nothing and shows finishConfirm', async (_l, me, teller) => {
    const commands = fakeCommands();
    const onDone = vi.fn();
    mount(() => <TicketActions ticket={ticket('SERVING', teller)} me={me} commands={commands} onDone={onDone} />);
    expect(button('finishConfirm')).toBeNull();
    button('finish')!.click();
    await flush();
    expect(commands.ticket).not.toHaveBeenCalled();
    expect(onDone).not.toHaveBeenCalled();
    expect(button('finishConfirm')?.textContent?.trim()).toBe('ยืนยันเสร็จสิ้น');
  });

  it('T1a-2 finishConfirm sends one finish for the ticket, then onDone once; a second click sends nothing', async () => {
    const commands = fakeCommands();
    let answer!: () => void;
    commands.ticket.mockImplementation(() => new Promise((r) => (answer = () => r({} as never))));
    const onDone = vi.fn();
    mount(() => <TicketActions ticket={ticket('SERVING', 'u-me')} me={ME} commands={commands} onDone={onDone} />);
    button('finish')!.click();
    await flush();
    expect(commands.ticket).not.toHaveBeenCalled();
    expect(button('finishConfirm')).not.toBeNull();
    button('finishConfirm')!.click();
    button('finishConfirm')?.click();
    await flush();
    expect(commands.ticket).toHaveBeenCalledTimes(1);
    expect(commands.ticket).toHaveBeenCalledWith('finish', 't-1');
    answer();
    await flush();
    expect(onDone).toHaveBeenCalledTimes(1);
  });

  it('T1a-2b en: finishConfirm reads Confirm finish', async () => {
    setLang('en');
    mount(() => <TicketActions ticket={ticket('SERVING', 'u-me')} me={ME} commands={fakeCommands()} onDone={() => {}} />);
    button('finish')!.click();
    await flush();
    expect(button('finishConfirm')?.textContent?.trim()).toBe('Confirm finish');
  });
});

describe('TicketActions: alreadyServing has its own wording (FE-T1a W3)', () => {
  it('T1a-3 th: a 409 alreadyServing on call shows the W3 text, not error.serverError', async () => {
    const commands = fakeCommands();
    commands.ticket.mockRejectedValue(new ApiError({ status: 409, code: 'alreadyServing', message: 'alreadyServing' }));
    const onDone = vi.fn();
    mount(() => <TicketActions ticket={ticket('WAITING', null)} me={ME} commands={commands} onDone={onDone} />);
    button('call')!.click();
    await flush();
    const text = host.querySelector('[role=alert]')?.textContent ?? '';
    expect(text).toContain(W3_TH);
    expect(text).not.toContain(sharedTh['error.serverError']);
    expect(onDone).not.toHaveBeenCalled();
  });

  it('T1a-3b en: a 409 alreadyServing on call shows the W3 text, not error.serverError', async () => {
    setLang('en');
    const commands = fakeCommands();
    commands.ticket.mockRejectedValue(new ApiError({ status: 409, code: 'alreadyServing', message: 'alreadyServing' }));
    mount(() => <TicketActions ticket={ticket('WAITING', null)} me={ME} commands={commands} onDone={() => {}} />);
    button('call')!.click();
    await flush();
    const text = host.querySelector('[role=alert]')?.textContent ?? '';
    expect(text).toContain(W3_EN);
    expect(text).not.toContain(sharedEn['error.serverError']);
  });
});
