// #5641 (DESK-SHIFT T4, docs/design/desk-shift-open.md §3 T4 rows G1–G10): which part of the shift flow /desk shows.
// Written before the implementation by the ticket author claude/studio-claude-15. Do not edit, delete or weaken;
// add tests in a file of your own.
import { describe, expect, it } from 'vitest';
import { makeCashCountRow, makeMe, makeStaffFloorView, makeStaffMeView } from '../../../shared/testing/fixtures';
import { deskShiftPhase } from './phase';

const teller = makeMe({ id: 'u-1', permissions: ['ShiftOperate', 'AttendanceClock'] });
const owner = makeMe({ id: 'u-o', permissions: ['ReversalRequest', 'OverviewRead'] });
const staffMe = makeStaffMeView({ userId: 'u-1', branchId: 'MAIN', permissions: ['ShiftOperate'] });
const floor = (over: Parameters<typeof makeStaffFloorView>[0] = {}) =>
  makeStaffFloorView({ phase: 'OPEN', hours: null, myClockIn: '2026-10-30T01:00:00Z', myShift: null, cutoff: null, target: null, safe: null, ...over });
const waitingOpen = makeCashCountRow({ id: 'c-1', kind: 'OPEN', countedBy: 'u-1' });

const base = {
  sessionMe: teller,
  staffMe,
  staffError: undefined as string | undefined,
  floor: floor(),
  countsLoaded: true,
  openWaiting: undefined as ReturnType<typeof makeCashCountRow> | undefined,
};

describe('#5641 deskShiftPhase', () => {
  it('G1 no session me yet: loading', () => {
    expect(deskShiftPhase({ ...base, sessionMe: undefined, staffMe: undefined, floor: undefined })).toBe('loading');
  });

  it('G4 no ShiftOperate: noAccess, before anything staff-only, even with a floor showing a shift', () => {
    expect(deskShiftPhase({ ...base, sessionMe: owner, staffMe: undefined, floor: undefined })).toBe('noAccess');
    expect(deskShiftPhase({ ...base, sessionMe: owner, floor: floor({ myShift: { id: 'SH-1', openedAt: 'x' } }) })).toBe('noAccess');
  });

  it('G2 / G2b staff me not read: error with an error, loading without', () => {
    expect(deskShiftPhase({ ...base, staffMe: undefined, floor: undefined, staffError: 'boom' })).toBe('error');
    expect(deskShiftPhase({ ...base, staffMe: undefined, floor: undefined })).toBe('loading');
  });

  it('G3 an open shift: open, whatever the counts say', () => {
    expect(
      deskShiftPhase({ ...base, floor: floor({ myShift: { id: 'SH-1', openedAt: 'x' } }), countsLoaded: false, openWaiting: waitingOpen }),
    ).toBe('open');
  });

  it('G5 / G6 no floor: error with an error, loading without', () => {
    expect(deskShiftPhase({ ...base, floor: undefined, staffError: 'boom' })).toBe('error');
    expect(deskShiftPhase({ ...base, floor: undefined })).toBe('loading');
  });

  it('G7 not clocked in: clockIn', () => {
    expect(deskShiftPhase({ ...base, floor: floor({ myClockIn: null }) })).toBe('clockIn');
  });

  it('G8 counts not loaded yet: loading (no count button before the first read)', () => {
    expect(deskShiftPhase({ ...base, countsLoaded: false })).toBe('loading');
  });

  it('G9 my OPEN count waits: waiting', () => {
    expect(deskShiftPhase({ ...base, openWaiting: waitingOpen })).toBe('waiting');
  });

  it('G10 otherwise: openCount', () => {
    expect(deskShiftPhase(base)).toBe('openCount');
  });
});
