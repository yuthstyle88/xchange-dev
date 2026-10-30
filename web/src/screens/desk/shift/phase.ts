import type { CashCountRow } from '../../../shared/api/generated/cash_counts';
import type { Me } from '../../../shared/api/generated/common';
import type { StaffFloor, StaffMe } from '../../staff/cards';
import { can } from '../../staff/parts/can';

export type DeskShiftPhase = 'loading' | 'error' | 'noAccess' | 'clockIn' | 'openCount' | 'waiting' | 'open';

export function deskShiftPhase(i: {
  sessionMe: Me | undefined;
  staffMe: StaffMe | undefined;
  staffError: string | undefined;
  floor: StaffFloor | undefined;
  countsLoaded: boolean;
  openWaiting: CashCountRow | undefined;
}): DeskShiftPhase {
  if (!i.sessionMe) return 'loading';
  if (!can(i.sessionMe, 'ShiftOperate')) return 'noAccess';
  if (!i.staffMe) return i.staffError ? 'error' : 'loading';
  if (i.floor?.myShift) return 'open';
  if (!i.floor) return i.staffError ? 'error' : 'loading';
  if (!i.floor.myClockIn) return 'clockIn';
  if (!i.countsLoaded) return 'loading';
  if (i.openWaiting) return 'waiting';
  return 'openCount';
}
