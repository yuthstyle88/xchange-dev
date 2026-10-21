import type { ApiClient } from './client';
import type { StaffCancelReason } from '../ui/CancelReasonDialog';

export interface KioskIdentityAnswer {
  id: string;
  queue: string;
  orderNo: string | null;
  transactionNo: string | null;
  status: string;
}

export function cancelKioskRequest(
  api: Pick<ApiClient, 'post'>,
  input: { id: string; branchId: string; reason: StaffCancelReason; note?: string },
): Promise<KioskIdentityAnswer> {
  const payload: { id: string; branchId: string; reason: StaffCancelReason; note?: string } = {
    id: input.id,
    branchId: input.branchId,
    reason: input.reason,
  };
  if (input.note !== undefined) {
    payload.note = input.note;
  }
  return api.post('/api/kiosk', { action: 'cancel', payload });
}
