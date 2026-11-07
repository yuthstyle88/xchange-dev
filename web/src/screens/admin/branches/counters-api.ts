// XC Counter (docs/design/xc-counter-app.md §8.4): the wire of Admin → สาขา → เคาน์เตอร์.

export type {
  BranchCashFromBrowserSaveAnswer,
  BranchCashFromBrowserSaveInput,
  BranchCounterRow,
  BranchCounterSaveAnswer,
  BranchCounterSaveInput,
  BranchCountersData,
} from '../../../shared/api/generated/branch_counters';

/** null = not set (§5.2). The resolved meaning of each is in §4: cashDrawer off, scanner off, kioskLock on. */
export type CounterSetting = boolean | null;

/** View `branch_counters`, `?branch=`. `cashFromBrowserVersion` is 0 when the setting row does not exist. */
export const BRANCH_COUNTERS_VIEW = 'branch_counters';

export const BRANCH_COUNTER_SAVE = '/api/v3/commands/branch_counter_save';

export const BRANCH_CASH_FROM_BROWSER_SAVE = '/api/v3/commands/branch_cash_from_browser_save';

