import { describe, expect, it, vi } from 'vitest';

export const STALE_KNOWN_FAILS = false;

// WE5: KNOWN list frozen at the 50 files that offend on origin/main df6e6927 / WE-1 branch head.
// Sorted alphabetically, one path per line (design §4.1).
export const KNOWN: readonly string[] = [
  'web/src/screens/admin/AdminShell.tsx',
  'web/src/screens/admin/alert-settings/AlertSettings.tsx',
  'web/src/screens/admin/approvals/Approvals.tsx',
  'web/src/screens/admin/bookings/BookingsScreen.tsx',
  'web/src/screens/admin/branches/BranchesScreen.tsx',
  'web/src/screens/admin/campaign-performance/CampaignPerformance.tsx',
  'web/src/screens/admin/campaigns/Campaigns.tsx',
  'web/src/screens/admin/cash-policy/CashPolicy.tsx',
  'web/src/screens/admin/cash/Cash.tsx',
  'web/src/screens/admin/cash/CashFlows.tsx',
  'web/src/screens/admin/cash/calculators/ProfitSummaryPanel.tsx',
  'web/src/screens/admin/cash/calculators/ShiftValuationPanel.tsx',
  'web/src/screens/admin/consent/Consent.tsx',
  'web/src/screens/admin/customers/Customers.tsx',
  'web/src/screens/admin/denomination-groups/DenominationGroups.tsx',
  'web/src/screens/admin/employees/Employees.tsx',
  'web/src/screens/admin/executive/Executive.tsx',
  'web/src/screens/admin/overview/Overview.tsx',
  'web/src/screens/admin/parts/NewTransactionButton.tsx',
  'web/src/screens/admin/parts/TransactionDetail.tsx',
  'web/src/screens/admin/parts/pending-write.tsx',
  'web/src/screens/admin/payments/Payments.tsx',
  'web/src/screens/admin/rate-alert-settings/RateAlertSettings.tsx',
  'web/src/screens/admin/rates/Rates.tsx',
  'web/src/screens/admin/reconciliation/Reconciliation.tsx',
  'web/src/screens/admin/reports/Reports.tsx',
  'web/src/screens/admin/slips/Slips.tsx',
  'web/src/screens/admin/starting-prices/StartingPrices.tsx',
  'web/src/screens/admin/transactions/Transactions.tsx',
  'web/src/screens/admin/users/UsersScreen.tsx',
  'web/src/screens/customer/assistant/AssistantScreen.tsx',
  'web/src/screens/customer/rate-alerts/RateAlerts.tsx',
  'web/src/screens/customer/sign-in/SignUp.tsx',
  'web/src/screens/desk-transaction/TransactionScreen.tsx',
  'web/src/screens/desk/referral/ReferralCodeField.tsx',
  'web/src/screens/desk/steps/customer/step.tsx',
  'web/src/screens/devices/DevicesScreen.tsx',
  'web/src/screens/documents/DocumentsScreen.tsx',
  'web/src/screens/documents/refusal.ts',
  'web/src/screens/kiosk-currencies/KioskCurrenciesScreen.tsx',
  'web/src/screens/staff-report/StaffReport.tsx',
  'web/src/screens/staff/StaffPage.tsx',
  'web/src/screens/staff/attendance/AttendanceCard.tsx',
  'web/src/screens/staff/floor/FloorCard.tsx',
  'web/src/screens/staff/holds/HoldsCard.tsx',
  'web/src/screens/staff/rates/RatesCard.tsx',
  'web/src/screens/staff/shift/ShiftCard.tsx',
  'web/src/screens/staff/transfers/TransfersCard.tsx',
  'web/src/shared/notifications/notifications.ts',
  'web/src/shared/ui/QueuePanel.tsx',
];

export function normalizePath(p: string): string {
  const norm = p.replace(/\\/g, '/');
  if (norm.startsWith('web/')) {
    return norm;
  }
  if (norm.startsWith('/src/')) {
    return 'web' + norm;
  }
  if (norm.startsWith('src/')) {
    return 'web/' + norm;
  }
  return 'web/src/' + norm.replace(/^\//, '');
}

export function shouldScanFile(filePath: string): boolean {
  const norm = filePath.replace(/\\/g, '/');
  if (!norm.endsWith('.ts') && !norm.endsWith('.tsx')) {
    return false;
  }
  if (norm.endsWith('.test.ts') || norm.endsWith('.test.tsx')) {
    return false;
  }
  if (norm.endsWith('worded-error.ts')) {
    return false;
  }
  // WE5: web/src/shared/ui/form.ts is excluded:
  // useForm receives an ApiError refusal already split into {code, message}, and shows
  // it through errorText by design (xchange-v3-web.md §5.2: useForm --> I18n : errorText(code, message)).
  // That is W10 (4) itself, not a copy of it.
  if (norm.endsWith('shared/ui/form.ts')) {
    return false;
  }
  return true;
}

export function isOffending(filePath: string, content: string): boolean {
  // Signal 1: String(err), String(e) or String(error)
  if (/String\((?:err|e|error)\)/.test(content)) {
    return true;
  }
  // Signal 2: definition of toWordedError, toErrorText or refusalText
  if (
    /(?:function|const|let|var)\s+(?:toWordedError|toErrorText|refusalText)\b|\basync\s+function\s+(?:toWordedError|toErrorText|refusalText)\b|\b(?:toWordedError|toErrorText|refusalText)\s*[:=]/.test(
      content,
    )
  ) {
    return true;
  }
  // Signal 3: any errorText( call outside web/src/shared/i18n/
  const isInsideI18n = filePath.includes('shared/i18n/');
  if (!isInsideI18n && /errorText\(/.test(content)) {
    return true;
  }
  return false;
}

export interface GateCheckResult {
  offenders: string[];
  newOffenders: string[];
  staleKnown: string[];
  passed: boolean;
  errors: string[];
}

export function checkGate(
  files: Record<string, string>,
  known: readonly string[],
  staleKnownFails: boolean,
): GateCheckResult {
  const offenders: string[] = [];
  for (const [rawPath, content] of Object.entries(files)) {
    const filePath = normalizePath(rawPath);
    if (!shouldScanFile(filePath)) {
      continue;
    }
    if (isOffending(filePath, content)) {
      offenders.push(filePath);
    }
  }
  offenders.sort();

  const normalizedKnown = known.map(normalizePath);
  const knownSet = new Set(normalizedKnown);
  const offendersSet = new Set(offenders);

  const newOffenders = offenders.filter((f) => !knownSet.has(f));
  const staleKnown = normalizedKnown.filter((f) => !offendersSet.has(f));

  const errors: string[] = [];
  if (newOffenders.length > 0) {
    errors.push(`New offending files found not in KNOWN:\n  ${newOffenders.join('\n  ')}`);
  }
  if (staleKnownFails && staleKnown.length > 0) {
    errors.push(`Clean files still in KNOWN with STALE_KNOWN_FAILS=true:\n  ${staleKnown.join('\n  ')}`);
  }
  if (!staleKnownFails && staleKnown.length > 0) {
    console.warn('Files in KNOWN that no longer offend:\n  ' + staleKnown.join('\n  '));
  }

  return {
    offenders,
    newOffenders,
    staleKnown,
    passed: errors.length === 0,
    errors,
  };
}

describe('WE5 worded-error gate (real source tree)', () => {
  it('KNOWN is sorted', () => {
    expect([...KNOWN].sort()).toEqual(KNOWN);
  });

  it('no new offending files outside of KNOWN exist in web/src', () => {
    const realFiles = import.meta.glob<string>('/src/**/*.{ts,tsx}', {
      query: '?raw',
      import: 'default',
      eager: true,
    });
    const result = checkGate(realFiles, KNOWN, STALE_KNOWN_FAILS);

    expect(result.newOffenders).toEqual([]);
    expect(result.passed).toBe(true);
  });
});

describe('WE5 gate unit tests (fixtures)', () => {
  it('a new offender fails', () => {
    const fixtures: Record<string, string> = {
      'web/src/screens/admin/AdminShell.tsx': 'const x = String(err);',
      'web/src/screens/new/NewScreen.tsx': 'const msg = String(e);',
    };
    const known = ['web/src/screens/admin/AdminShell.tsx'];
    const result = checkGate(fixtures, known, false);

    expect(result.passed).toBe(false);
    expect(result.newOffenders).toEqual(['web/src/screens/new/NewScreen.tsx']);
    expect(result.errors.length).toBeGreaterThan(0);
  });

  it('a clean KNOWN file passes while the flag is false', () => {
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const fixtures: Record<string, string> = {
      'web/src/screens/admin/AdminShell.tsx': 'const x = wordedError(err);',
    };
    const known = ['web/src/screens/admin/AdminShell.tsx'];
    const result = checkGate(fixtures, known, false);

    expect(result.passed).toBe(true);
    expect(result.staleKnown).toEqual(['web/src/screens/admin/AdminShell.tsx']);
    expect(warnSpy).toHaveBeenCalled();
    warnSpy.mockRestore();
  });

  it('the same file fails when the flag is true', () => {
    const fixtures: Record<string, string> = {
      'web/src/screens/admin/AdminShell.tsx': 'const x = wordedError(err);',
    };
    const known = ['web/src/screens/admin/AdminShell.tsx'];
    const result = checkGate(fixtures, known, true);

    expect(result.passed).toBe(false);
    expect(result.staleKnown).toEqual(['web/src/screens/admin/AdminShell.tsx']);
    expect(result.errors.some((e) => e.includes('STALE_KNOWN_FAILS=true'))).toBe(true);
  });

  it('each of the three signals is detected', () => {
    // Signal 1: String(err), String(e), String(error)
    expect(isOffending('web/src/screens/A.tsx', 'const msg = String(err);')).toBe(true);
    expect(isOffending('web/src/screens/A.tsx', 'const msg = String(e);')).toBe(true);
    expect(isOffending('web/src/screens/A.tsx', 'const msg = String(error);')).toBe(true);
    expect(isOffending('web/src/screens/A.tsx', 'const msg = String(other);')).toBe(false);

    // Signal 2: definition of toWordedError, toErrorText or refusalText
    expect(isOffending('web/src/screens/A.tsx', 'function toWordedError(err: unknown) { return ""; }')).toBe(true);
    expect(isOffending('web/src/screens/A.tsx', 'const toErrorText = (e: unknown) => "";')).toBe(true);
    expect(isOffending('web/src/screens/A.tsx', 'export function refusalText(err: unknown) { return ""; }')).toBe(true);
    expect(isOffending('web/src/screens/A.tsx', 'const helper = (err: unknown) => "";')).toBe(false);

    // Signal 3: any errorText( call outside web/src/shared/i18n/
    expect(isOffending('web/src/screens/A.tsx', 'return errorText(code, msg);')).toBe(true);
    expect(isOffending('web/src/shared/i18n/index.ts', 'return errorText(code, msg);')).toBe(false);
    expect(isOffending('web/src/shared/i18n/worded-error.ts', 'return errorText(code, msg);')).toBe(false);
  });

  it('form.ts and test files are not scanned', () => {
    // Form and test files should be excluded by shouldScanFile
    expect(shouldScanFile('web/src/shared/ui/form.ts')).toBe(false);
    expect(shouldScanFile('src/shared/ui/form.ts')).toBe(false);
    expect(shouldScanFile('/src/shared/ui/form.ts')).toBe(false);
    expect(shouldScanFile('web/src/screens/admin/AdminShell.test.tsx')).toBe(false);
    expect(shouldScanFile('web/src/shared/i18n/worded-error.test.ts')).toBe(false);
    expect(shouldScanFile('web/src/shared/i18n/worded-error.gate.test.ts')).toBe(false);
    expect(shouldScanFile('web/src/shared/i18n/worded-error.ts')).toBe(false);

    // Non-excluded source files are scanned
    expect(shouldScanFile('web/src/screens/admin/AdminShell.tsx')).toBe(true);

    // In checkGate, even if form.ts and test files contain signals, they are ignored
    const fixtures: Record<string, string> = {
      'web/src/shared/ui/form.ts': 'errorText(code, msg); String(err);',
      'web/src/screens/admin/AdminShell.test.tsx': 'String(err); toWordedError();',
      'web/src/shared/i18n/worded-error.ts': 'errorText(code, msg);',
    };
    const result = checkGate(fixtures, [], false);
    expect(result.offenders).toEqual([]);
    expect(result.passed).toBe(true);
  });
});
