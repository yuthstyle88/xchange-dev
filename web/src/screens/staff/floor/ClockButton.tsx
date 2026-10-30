import type { Accessor, JSX } from 'solid-js';
import { screenT } from '../../../shared/i18n';
import { wordedError } from '../../../shared/i18n/worded-error';
import { ApiClient } from '../../../shared/api/client';
import type {
  AttendanceClockInput,
  AttendanceClockOutput,
} from '../../../shared/api/generated/branch_day';
import type { StaffCtx } from '../cards';
import { command } from '../parts/command';
import { th, en } from './i18n';

export interface ClockButtonProps {
  ctx: StaffCtx;
  busy: Accessor<boolean>;
  setBusy(v: boolean): void;
  onNotice(msg: string | null): void;
  onError(msg: string | null): void;
}

export function ClockButton(p: ClockButtonProps): JSX.Element {
  const t = screenT('staff.floor', { th, en });
  const api = new ApiClient();
  const clockCmd = command<AttendanceClockInput, AttendanceClockOutput>(
    api,
    'POST',
    '/api/attendance',
  );

  const handleClock = async () => {
    if (p.busy()) return;
    p.setBusy(true);
    try {
      p.onError(null);
      const direction = p.ctx.floor?.myClockIn ? 'OUT' : 'IN';
      await clockCmd.send({
        branch: p.ctx.me.branchId,
        direction,
      });
      p.onNotice(t('saved'));
      await p.ctx.refresh();
    } catch (e: unknown) {
      p.onNotice(null);
      p.onError(wordedError(e));
    } finally {
      p.setBusy(false);
    }
  };

  return (
    <button
      type="button"
      data-testid="floor-clock"
      class="xc-floor-clock-btn"
      disabled={p.busy()}
      onClick={handleClock}
    >
      {p.ctx.floor?.myClockIn ? t('clockOut') : t('clockIn')}
    </button>
  );
}
