// #5569 (DESK-R4 T4, #5541 FE-1; docs/design/fe-desk-round4.md §3 T4, queue-tickets.md §4.6): the "ไม่มีการขาย"
// release of a SERVING ticket. Written before the implementation by the ticket author claude/studio-claude-15.
// Do not edit, delete or weaken; add tests in a file of your own.
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { ApiError } from '../../../../shared/api/client';
import { setLang } from '../../../../shared/i18n';
import { TicketActions } from './TicketActions';
import { en, th } from './i18n';

const ME = { userId: 'u-me', permissions: ['QueueOperate'] };
const MANAGER = { userId: 'u-boss', permissions: ['QueueOperate', 'BranchDayOperate'] };
const ticket = (status: string, tellerId: string | null) => ({ id: 't-1', displayNumber: 'W013', status, tellerId });

function fakeCommands() {
  const answer = { id: 't-1', displayNumber: 'W013', serviceType: 'WALK_IN', sequence: 13, status: 'CANCELLED', tellerId: 'u-me', endedReason: 'noSale' };
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

describe('TicketActions: release with no sale (#5569)', () => {
  it('R0 the texts are Thai, and en carries the same Thai', () => {
    const t = th as Record<string, string>;
    const e = en as Record<string, string>;
    expect(t.release).toBe('ไม่มีการขาย');
    expect(t.releaseConfirm).toBe('ยืนยันไม่มีการขาย');
    expect(t['error.ticketHasSale']).toBe('คิวนี้มีรายการขายแล้ว กรุณายกเลิกรายการขายก่อน');
    for (const k of ['release', 'releaseConfirm', 'error.ticketHasSale']) expect(e[k]).toBe(t[k]);
  });

  it.each([
    ['the serving teller', ME, 'u-me'],
    ['a branch manager', MANAGER, 'u-other'],
  ] as const)('R1 SERVING, %s: a release button reading ไม่มีการขาย, beside finish', (_l, me, teller) => {
    mount(() => <TicketActions ticket={ticket('SERVING', teller)} me={me} commands={fakeCommands()} onDone={() => {}} />);
    expect(button('finish')).not.toBeNull();
    expect(button('release')?.textContent?.trim()).toBe('ไม่มีการขาย');
  });

  it.each(['WAITING', 'CALLED'])('R1b %s: no release button', (status) => {
    mount(() => <TicketActions ticket={ticket(status, 'u-me')} me={ME} commands={fakeCommands()} onDone={() => {}} />);
    expect(button('release')).toBeNull();
  });

  it('R2 a click on release posts nothing and shows the confirm button', async () => {
    const commands = fakeCommands();
    mount(() => <TicketActions ticket={ticket('SERVING', 'u-me')} me={ME} commands={commands} onDone={() => {}} />);
    button('release')!.click();
    await flush();
    expect(commands.ticket).not.toHaveBeenCalled();
    expect(button('releaseConfirm')?.textContent?.trim()).toBe('ยืนยันไม่มีการขาย');
  });

  it('R3 confirm sends one release for the ticket, then onDone once; a second click sends nothing', async () => {
    const commands = fakeCommands();
    let answer!: () => void;
    commands.ticket.mockImplementation(() => new Promise((r) => (answer = () => r({} as never))));
    const onDone = vi.fn();
    mount(() => <TicketActions ticket={ticket('SERVING', 'u-me')} me={ME} commands={commands} onDone={onDone} />);
    button('release')!.click();
    await flush();
    button('releaseConfirm')!.click();
    button('releaseConfirm')?.click();
    await flush();
    expect(commands.ticket).toHaveBeenCalledTimes(1);
    expect(commands.ticket).toHaveBeenCalledWith('release', 't-1');
    answer();
    await flush();
    expect(onDone).toHaveBeenCalledTimes(1);
  });

  it('R4 a ticketHasSale refusal shows its own Thai line and does not call onDone', async () => {
    const commands = fakeCommands();
    commands.ticket.mockRejectedValue(new ApiError({ status: 409, code: 'ticketHasSale', message: 'refused' }));
    const onDone = vi.fn();
    mount(() => <TicketActions ticket={ticket('SERVING', 'u-me')} me={ME} commands={commands} onDone={onDone} />);
    button('release')!.click();
    await flush();
    button('releaseConfirm')!.click();
    await flush();
    expect(host.querySelector('[role=alert]')?.textContent).toContain('คิวนี้มีรายการขายแล้ว กรุณายกเลิกรายการขายก่อน');
    expect(onDone).not.toHaveBeenCalled();
  });

  it.each(['notYourTicket', 'noOpenShift', 'staleVersion'])('R5 a %s refusal on release shows its own text', async (code) => {
    const commands = fakeCommands();
    commands.ticket.mockRejectedValue(new ApiError({ status: 409, code, message: 'refused' }));
    mount(() => <TicketActions ticket={ticket('SERVING', 'u-me')} me={ME} commands={commands} onDone={() => {}} />);
    button('release')!.click();
    await flush();
    button('releaseConfirm')!.click();
    await flush();
    expect(host.querySelector('[role=alert]')?.textContent).toContain((th as Record<string, string>)[`error.${code}`]);
  });

  it("R6 SERVING, another teller's ticket: no release button", () => {
    mount(() => <TicketActions ticket={ticket('SERVING', 'u-other')} me={ME} commands={fakeCommands()} onDone={() => {}} />);
    expect(button('release')).toBeNull();
  });
});
