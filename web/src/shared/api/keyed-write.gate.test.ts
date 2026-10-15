import { describe, expect, it, vi } from 'vitest';

export const STALE_KNOWN_FAILS = false;

// KW-gate (#2044, docs/design/xchange-v3-web.md §12.24): frozen at the 27 offending files on origin/main
// outside keyed-write.ts and the 3 named exclusions. Sorted alphabetically, one path per line.
export const KNOWN: readonly string[] = [
  'web/src/screens/admin/alert-settings/AlertSettings.tsx',
  'web/src/screens/admin/approvals/Approvals.tsx',
  'web/src/screens/admin/booking-slots/BookingSlots.tsx',
  'web/src/screens/admin/branches/BranchesScreen.tsx',
  'web/src/screens/admin/campaigns/Campaigns.tsx',
  'web/src/screens/admin/cash-policy/CashPolicy.tsx',
  'web/src/screens/admin/cash/CashFlows.tsx',
  'web/src/screens/admin/consent/Consent.tsx',
  'web/src/screens/admin/denomination-groups/DenominationGroups.tsx',
  'web/src/screens/admin/members/Members.tsx',
  'web/src/screens/admin/rate-alert-settings/RateAlertSettings.tsx',
  'web/src/screens/admin/rates/Rates.tsx',
  'web/src/screens/admin/reconciliation/Reconciliation.tsx',
  'web/src/screens/admin/starting-prices/StartingPrices.tsx',
  'web/src/screens/admin/users/UsersScreen.tsx',
  'web/src/screens/customer/assistant/AssistantScreen.tsx',
  'web/src/screens/customer/book/Book.tsx',
  'web/src/screens/customer/bookings/Bookings.tsx',
  'web/src/screens/customer/home/HomeScreen.tsx',
  'web/src/screens/customer/rate-alerts/RateAlerts.tsx',
  'web/src/screens/desk/bookings/DeskBookingsScreen.tsx',
  'web/src/screens/desk/referral/ReferralCodeField.tsx',
  'web/src/screens/desk/save.ts',
  'web/src/screens/devices/DevicesScreen.tsx',
  'web/src/screens/devices/main.tsx',
  'web/src/screens/operations/verification/VerificationScreen.tsx',
  'web/src/shared/notifications/notifications.ts',
];

export function normalizePath(p: string): string {
  const norm = p.replace(/\\/g, '/').replace(/^\//, '');
  if (norm.startsWith('web/')) {
    return norm;
  }
  if (norm.startsWith('src/')) {
    return 'web/' + norm;
  }
  if (
    norm.startsWith('admin/') ||
    norm.startsWith('customer/') ||
    norm.startsWith('desk/') ||
    norm.startsWith('devices/') ||
    norm.startsWith('operations/') ||
    norm.startsWith('staff/')
  ) {
    return 'web/src/screens/' + norm;
  }
  return 'web/src/' + norm;
}

const EXCLUDED_FILES: ReadonlySet<string> = new Set([
  // §12.24: the shared idempotency-key core itself owns key generation.
  'web/src/shared/api/keyed-write.ts',
  // §12.24 named exclusion 1: requestId is a payload field of the admin pending-write protocol
  // (read back through a view filter, :214), not an Idempotency-Key header.
  'web/src/screens/admin/parts/pending-write.tsx',
  // §12.24 named exclusion 2: memberSecret is a secret value, not an Idempotency-Key header.
  'web/src/screens/admin/customers/Customers.tsx',
  // §12.24 named exclusion 3: the v2 bookings screen, deleted by booking-slots.md BK-11.
  'web/src/screens/admin/bookings/BookingsScreen.tsx',
]);

export function shouldScanFile(filePath: string): boolean {
  const norm = normalizePath(filePath);
  if (!norm.endsWith('.ts') && !norm.endsWith('.tsx')) {
    return false;
  }
  if (norm.endsWith('.test.ts') || norm.endsWith('.test.tsx')) {
    return false;
  }
  if (EXCLUDED_FILES.has(norm)) {
    return false;
  }
  return true;
}

const RANDOM_UUID_RE = /\bcrypto\.randomUUID\s*\(/;

export function isOffending(content: string): boolean {
  return RANDOM_UUID_RE.test(content);
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
    if (isOffending(content)) {
      offenders.push(filePath);
    }
  }
  offenders.sort();

  const normalizedKnown = known.map(normalizePath);
  const sortedKnown = [...normalizedKnown].sort();
  const isKnownSorted = normalizedKnown.every((item, idx) => item === sortedKnown[idx]);

  const knownSet = new Set(normalizedKnown);
  const offendersSet = new Set(offenders);

  const newOffenders = offenders.filter((f) => !knownSet.has(f));
  const staleKnown = normalizedKnown.filter((f) => !offendersSet.has(f));

  const errors: string[] = [];
  if (!isKnownSorted) {
    errors.push('KNOWN list must be sorted alphabetically');
  }
  if (newOffenders.length > 0) {
    errors.push(`New offending files found not in KNOWN:\n  ${newOffenders.join('\n  ')}`);
  }
  if (staleKnownFails && staleKnown.length > 0) {
    errors.push(
      `Clean files still in KNOWN with STALE_KNOWN_FAILS=true:\n  ${staleKnown.join('\n  ')}`,
    );
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

describe('KW-gate keyed-write gate (real source tree)', () => {
  it('no new offending files outside of KNOWN exist in web/src', () => {
    const realFiles = import.meta.glob<string>('/src/**/*.{ts,tsx}', {
      query: '?raw',
      import: 'default',
      eager: true,
    });
    const result = checkGate(realFiles, KNOWN, STALE_KNOWN_FAILS);

    expect([...KNOWN].sort()).toEqual(KNOWN);
    expect(result.newOffenders).toEqual([]);
    expect(result.passed).toBe(true);
  });
});

describe('KW-gate keyed-write gate (§9 behaviour table)', () => {
  it("row 1: a new file containing crypto.randomUUID() fails with 'New offending files'", () => {
    expect(isOffending('const key = crypto.randomUUID();')).toBe(true);

    const fixtures: Record<string, string> = {
      'web/src/screens/new/NewScreen.tsx': 'const key = crypto.randomUUID();',
    };
    const result = checkGate(fixtures, [], false);

    expect(result.passed).toBe(false);
    expect(result.newOffenders).toEqual(['web/src/screens/new/NewScreen.tsx']);
    expect(result.errors.join('\n')).toContain('New offending files');
  });

  it('row 2: the same inside *.test.tsx passes (tests excluded)', () => {
    expect(shouldScanFile('web/src/screens/admin/approvals/Approvals.test.tsx')).toBe(false);
    expect(shouldScanFile('web/src/shared/api/keyed-write.test.ts')).toBe(false);

    const fixtures: Record<string, string> = {
      'web/src/screens/admin/approvals/Approvals.test.tsx': 'const key = crypto.randomUUID();',
      'web/src/shared/api/keyed-write.test.ts': 'const key = crypto.randomUUID();',
    };
    const result = checkGate(fixtures, [], false);

    expect(result.passed).toBe(true);
    expect(result.offenders).toEqual([]);
  });

  it('row 3: shared/api/keyed-write.ts passes (excluded)', () => {
    expect(shouldScanFile('shared/api/keyed-write.ts')).toBe(false);
    expect(shouldScanFile('web/src/shared/api/keyed-write.ts')).toBe(false);
    expect(shouldScanFile('/src/shared/api/keyed-write.ts')).toBe(false);

    const fixtures: Record<string, string> = {
      'shared/api/keyed-write.ts': 'const key = crypto.randomUUID();',
    };
    const result = checkGate(fixtures, [], false);

    expect(result.passed).toBe(true);
    expect(result.offenders).toEqual([]);
  });

  it('row 4: admin/parts/pending-write.tsx passes (named exclusion)', () => {
    expect(shouldScanFile('admin/parts/pending-write.tsx')).toBe(false);
    expect(shouldScanFile('web/src/screens/admin/parts/pending-write.tsx')).toBe(false);
    expect(shouldScanFile('admin/customers/Customers.tsx')).toBe(false);
    expect(shouldScanFile('admin/bookings/BookingsScreen.tsx')).toBe(false);

    const fixtures: Record<string, string> = {
      'admin/parts/pending-write.tsx': 'const requestId = crypto.randomUUID();',
      'admin/customers/Customers.tsx': 'const memberSecret = crypto.randomUUID();',
      'admin/bookings/BookingsScreen.tsx': 'const key = crypto.randomUUID();',
    };
    const result = checkGate(fixtures, [], false);

    expect(result.passed).toBe(true);
    expect(result.offenders).toEqual([]);
  });

  it('row 5: a KNOWN file now clean, flag false, warns and passes', () => {
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const fixtures: Record<string, string> = {
      'web/src/screens/admin/approvals/Approvals.tsx': 'const write = keyedWrite(api, "POST", "/x");',
    };
    const known = ['web/src/screens/admin/approvals/Approvals.tsx'];
    const result = checkGate(fixtures, known, false);

    expect(result.passed).toBe(true);
    expect(result.staleKnown).toEqual(['web/src/screens/admin/approvals/Approvals.tsx']);
    expect(warnSpy).toHaveBeenCalled();
    warnSpy.mockRestore();
  });

  it("row 6: the same, flag true, fails with 'Clean files still in KNOWN'", () => {
    const fixtures: Record<string, string> = {
      'web/src/screens/admin/approvals/Approvals.tsx': 'const write = keyedWrite(api, "POST", "/x");',
    };
    const known = ['web/src/screens/admin/approvals/Approvals.tsx'];
    const result = checkGate(fixtures, known, true);

    expect(result.passed).toBe(false);
    expect(result.staleKnown).toEqual(['web/src/screens/admin/approvals/Approvals.tsx']);
    expect(result.errors.join('\n')).toContain('Clean files still in KNOWN');
  });

  it('row 7: KNOWN unsorted fails', () => {
    expect([...KNOWN].sort()).toEqual(KNOWN);

    const unsortedKnown = [
      'web/src/screens/admin/consent/Consent.tsx',
      'web/src/screens/admin/approvals/Approvals.tsx',
    ];
    const fixtures: Record<string, string> = {
      'web/src/screens/admin/approvals/Approvals.tsx': 'const k = crypto.randomUUID();',
      'web/src/screens/admin/consent/Consent.tsx': 'const k = crypto.randomUUID();',
    };
    const result = checkGate(fixtures, unsortedKnown, false);

    expect(result.passed).toBe(false);
    expect(result.errors.join('\n')).toContain('KNOWN list must be sorted');
  });
});
