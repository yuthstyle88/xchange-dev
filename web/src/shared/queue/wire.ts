// Hand-declared view types until V1-kiosk-queue-views (#485) merges.
// When #485 merges, this file will re-export from web/src/shared/api/generated/...
// Keep 64-bit integer fields (durationSeconds, count, workingCount) as number (§12.18.2, §12.18.11).

export interface KioskNotificationsWorking {
  id: string;
  queue: string;
  staff: string;
  startedAt: string | null;
  status: string;
}

export interface KioskNotifications {
  serverNow: string;
  branchId: string;
  branchName: string;
  count: number;
  latest: string | null;
  workingCount: number;
  working: KioskNotificationsWorking[];
  employee: {
    id: string;
    role: string;
  };
}

export interface KioskQueueRow {
  id: string;
  queue: string;
  orderNo: string;
  transactionNo: string | null;
  status: string;
  type: 'BUY' | 'SELL';
  currency: string;
  foreign: string;
  thb: string;
  staff: string;
  startedAt: string | null;
  completedAt: string | null;
  durationSeconds: number | null;
  canAccept: boolean;
  canContinue: boolean;
}

export interface KioskQueueFilter {
  state?: 'waiting' | 'working' | 'completed';
}
