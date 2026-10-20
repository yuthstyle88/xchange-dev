// QA-S1 — smoke: public pages (/login, /app/sign-in, /app/sign-up, /rates) (design docs/design/qa-e2e.md §3, §4). Written by ticket QA-S1; this file is its whole File scope.
// Import only from './helpers'. Never run against a host from a worker's machine (E-3): check with --list and tsc.
import { test } from '@playwright/test';

test.describe('QA-S1 smoke: public pages (/login, /app/sign-in, /app/sign-up, /rates)', () => {
  // filled by QA-S1
});
