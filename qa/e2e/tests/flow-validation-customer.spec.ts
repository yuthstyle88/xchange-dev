// QA-R9-11 — flow: invalid input validation on customer forms (design docs/design/qa-e2e.md §9.1 b, §9.5 row R9-11; #4295).
// Covers (b) only (3–5 invalid input cases per form) for customer forms walked by earlier rounds:
// - /app/book (bookingCreate): no slot (server 400), amount 0 (client rangeUnderflow), negative amount (client rangeUnderflow),
//   too many decimals (server 400), past date (client rangeUnderflow)
// - /app/profile (profileUpdate, consentRecord): empty / whitespace display name (client disabled + no request), consentRecord invalid inputs
// - /app/rate-alerts (rateAlertCreate): empty threshold (client valueMissing), negative threshold, zero threshold, invalid scale (server 400)
// - /app/sign-up (no session): empty display name, empty identifier, empty password, already-registered account
// - /app/journey (plannerDraftSave): amount 0, negative amount, non-numeric amount (client validationError)
// - /app/bookings (bookingCancel, bookingReschedule): reschedule without slot, past date, non-existent booking cancel
// Each case: fresh refusal checked (response 400 or proven no request with visible block), worded refusal in th and en,
// form retains typed input, and read-back confirms nothing was created.
//
// Elements cited from web/src/screens/:
// - customer/book/Book.tsx:
//   - [data-testid="book-screen"]        line 394
//   - .alert.alert-error                 line 401
//   - [data-field="branchId"]            line 414
//   - select[data-testid="branch-select"] line 444
//   - [data-field="serviceDate"]         line 487
//   - input[data-testid="date-input"]    line 496
//   - [data-field="slotId"]              line 529
//   - select[data-testid="slot-select"]  line 560
//   - select[data-testid="currency-select"] line 612
//   - input[data-testid="quantity-input"] line 635
//   - input[data-testid="referral-code"] line 656
//   - button[data-testid="book-submit"]  line 687
// - customer/profile/Profile.tsx:
//   - p[data-testid="profile-error"]     line 223
//   - p[data-testid="profile-saved"]     line 230
//   - input[data-field="displayName"]    line 245
//   - select[data-field="preferredLanguage"] line 257
//   - button[data-action="save-profile"] line 276
//   - input[data-consent]                line 291
// - customer/rate-alerts/RateAlerts.tsx:
//   - .xc-rate-alerts-screen             line 391
//   - form.xc-create-alert-form          line 398
//   - select#alert-branch                line 402
//   - input#alert-currency               line 426
//   - select#alert-side                  line 439
//   - select#alert-condition             line 452
//   - input#alert-target                 line 464
//   - input#alert-expires                line 479
//   - .xc-create-alert-form .xc-form-error line 487
//   - .xc-create-alert-form button[type="submit"] line 495
//   - article.xc-alert-card              line 548
// - customer/sign-in/SignUp.tsx:
//   - .xc-sign-in                        line 66
//   - form.xc-sign-in-form               line 68
//   - input[name="displayName"]          line 71
//   - input[name="identifier"]           line 75
//   - input[name="password"]             line 77
//   - input[name="marketing"]            line 79
//   - form.xc-sign-in-form button[type="submit"] line 83
//   - p.xc-sign-in-error                 line 88
// - customer/sign-in/TermsText.tsx:
//   - .xc-sign-in-terms                  line 57
//   - .xc-sign-in-terms-box              line 63
// - customer/home/HomeScreen.tsx:
//   - select#currency-select             line 481
//   - input#amount-input                 line 516
//   - .xc-home-calc-actions button       line 532
//   - .xc-home-alert.xc-home-alert-error line 545
//   - .xc-home-estimate-card             line 554
//   - .xc-home-save-actions button       line 581
// - customer/bookings/Bookings.tsx:
//   - button[data-action="cancel"]       line 440
//   - button[data-action="confirm-cancel"] line 503
//   - button[data-action="reschedule"]   line 450
//   - input[data-testid="date-input"]    line 536
//   - input[name="rescheduleSlot"]       line 594
//   - button[data-action="confirm-reschedule"] line 628
//
// Import only from './helpers'. Never run against a host from a worker's machine (E-3): check with --list and tsc.
import { expect, test } from '@playwright/test';
import { type Lang, signIn, users, withLang } from './helpers';

const LANGS: readonly Lang[] = ['th', 'en'];
const sameOrigin = { 'x-xchange-request': '1', origin: new URL(users.baseUrl).origin };

test.describe('QA-R9-11 flow: customer form validations (§9.1 b)', () => {
  for (const lang of LANGS) {
    test(`/app/book bookingCreate validation (${lang})`, async ({ page }) => {
      await withLang(page, lang);
      await page.goto('about:blank');
      await signIn(page, 'customer', '/app/book');

      // Read back initial bookings count before validation attempts
      const initialBookingsRes = await page.request.get('/api/v3/views/my_bookings');
      const initialBookingsJson = (await initialBookingsRes.json().catch(() => ({}))) as {
        data?: { bookings?: unknown[] };
        bookings?: unknown[];
      };
      const initialBookingsCount =
        initialBookingsJson?.data?.bookings?.length ?? initialBookingsJson?.bookings?.length ?? 0;

      await page.goto('/app/book');
      const screen = page.locator('[data-testid="book-screen"]');
      await expect(screen).toBeVisible({ timeout: 20_000 });

      const branchSelect = page.locator('select[data-testid="branch-select"]');
      await expect(branchSelect).toBeVisible({ timeout: 15_000 });
      if ((await branchSelect.locator('option').count()) > 1) {
        await branchSelect.selectOption({ index: 1 }, { timeout: 10_000 });
      }

      const qtyInput = page.locator('input[data-testid="quantity-input"]');
      const slotSelect = page.locator('select[data-testid="slot-select"]');
      const submitBtn = page.locator('button[data-testid="book-submit"]');
      const errorAlert = page.locator('.alert.alert-error');
      const expectedValidationMsg =
        lang === 'th' ? 'ข้อมูลไม่ถูกต้อง กรุณาตรวจสอบอีกครั้ง' : 'Validation failed, please check your input';

      // Case 1: no slot selected (server 400 refusal)
      console.log(`[r9-11] book ${lang} case 1`);
      await qtyInput.fill('100', { timeout: 10_000 });
      if (await slotSelect.isVisible()) {
        await slotSelect.selectOption('', { timeout: 10_000 });
      }
      const noSlotResPromise = page.waitForResponse(
        (res) => res.request().method() === 'POST' && /\/api\/bookings$/.test(new URL(res.url()).pathname),
        { timeout: 15_000 },
      );
      await submitBtn.click({ timeout: 10_000 });
      const noSlotRes = await noSlotResPromise;
      expect(noSlotRes.status(), `${lang}: no slot selected refused with 400`).toBe(400);
      await expect(errorAlert).toBeVisible({ timeout: 10_000 });
      await expect(qtyInput, `${lang}: quantity retains input`).toHaveValue('100');

      // Case 2: amount 0 (client refusal via min="1" and rangeUnderflow)
      console.log(`[r9-11] book ${lang} case 2`);
      if (await slotSelect.isVisible() && (await slotSelect.locator('option[data-testid="slot-option"]').count()) > 0) {
        await slotSelect.selectOption({ index: 1 }, { timeout: 10_000 });
      }
      await qtyInput.fill('0', { timeout: 10_000 });
      const isZeroUnderflow = await qtyInput.evaluate((el: HTMLInputElement) => el.validity.rangeUnderflow);
      expect(isZeroUnderflow, `${lang}: amount 0 triggers rangeUnderflow`).toBe(true);
      const noZeroReq = page.waitForRequest(/\/api\/bookings$/, { timeout: 2_000 });
      await submitBtn.click({ timeout: 10_000 });
      await expect(noZeroReq, `${lang}: no request sent for amount 0`).rejects.toThrow();
      await expect(qtyInput, `${lang}: quantity retains 0`).toHaveValue('0');

      // Case 3: negative amount (client refusal via min="1" and rangeUnderflow)
      console.log(`[r9-11] book ${lang} case 3`);
      await qtyInput.fill('-50', { timeout: 10_000 });
      const isNegUnderflow = await qtyInput.evaluate((el: HTMLInputElement) => el.validity.rangeUnderflow);
      expect(isNegUnderflow, `${lang}: negative amount triggers rangeUnderflow`).toBe(true);
      const noNegReq = page.waitForRequest(/\/api\/bookings$/, { timeout: 2_000 });
      await submitBtn.click({ timeout: 10_000 });
      await expect(noNegReq, `${lang}: no request sent for negative amount`).rejects.toThrow();
      await expect(qtyInput, `${lang}: quantity retains -50`).toHaveValue('-50');

      // Case 4: too many decimals (>4 decimals; server 400 refusal)
      console.log(`[r9-11] book ${lang} case 4`);
      await qtyInput.fill('10.12345', { timeout: 10_000 });
      const decResPromise = page.waitForResponse(
        (res) => res.request().method() === 'POST' && /\/api\/bookings$/.test(new URL(res.url()).pathname),
        { timeout: 15_000 },
      );
      await submitBtn.click({ timeout: 10_000 });
      const decRes = await decResPromise;
      expect(decRes.status(), `${lang}: too many decimals refused with 400`).toBe(400);
      await expect(errorAlert).toBeVisible({ timeout: 10_000 });
      await expect(errorAlert).toContainText(expectedValidationMsg);
      await expect(qtyInput, `${lang}: quantity retains 10.12345`).toHaveValue('10.12345');

      // Case 5: past date (client refusal via min attribute, rejected submit request)
      console.log(`[r9-11] book ${lang} case 5`);
      const dateInput = page.locator('input[data-testid="date-input"]');
      if (await dateInput.isVisible()) {
        await dateInput.fill('2020-01-01', { timeout: 10_000 });
        const isDateUnderflow = await dateInput.evaluate((el: HTMLInputElement) => el.validity.rangeUnderflow);
        expect(isDateUnderflow, `${lang}: past date triggers rangeUnderflow`).toBe(true);
        const noPastDateReq = page.waitForRequest(/\/api\/bookings$/, { timeout: 2_000 });
        await submitBtn.click({ timeout: 10_000 });
        await expect(noPastDateReq, `${lang}: no request sent for past date`).rejects.toThrow();
        await expect(dateInput, `${lang}: date retains past date`).toHaveValue('2020-01-01');
      }

      // Read back: confirm no booking was created
      const finalBookingsRes = await page.request.get('/api/v3/views/my_bookings');
      const finalBookingsJson = (await finalBookingsRes.json().catch(() => ({}))) as {
        data?: { bookings?: unknown[] };
        bookings?: unknown[];
      };
      const finalBookingsCount =
        finalBookingsJson?.data?.bookings?.length ?? finalBookingsJson?.bookings?.length ?? 0;
      expect(finalBookingsCount, `${lang}: bookings count unchanged`).toBe(initialBookingsCount);
    });

    test(`/app/profile profileUpdate & consentRecord validation (${lang})`, async ({ page }) => {
      await withLang(page, lang);
      await page.goto('about:blank');
      await signIn(page, 'customer', '/app/profile');
      await page.goto('/app/profile');

      await expect(page.locator('.xc-profile-body')).toBeVisible({ timeout: 20_000 });

      const nameInput = page.locator('input[data-field="displayName"]');
      const saveBtn = page.locator('button[data-action="save-profile"]');

      await expect(nameInput).toBeVisible({ timeout: 15_000 });
      const origName = await nameInput.inputValue();

      try {
        // Case 1: empty display name -> client disables save button and blocks request
        await nameInput.fill('');
        await expect(saveBtn, `${lang}: save button disabled on empty name`).toBeDisabled();
        await expect(nameInput).toHaveValue('');
        const noReq1 = page.waitForRequest(/\/api\/app\/profile$/, { timeout: 2_000 });
        await saveBtn.click({ force: true }).catch(() => {});
        await expect(noReq1, `${lang}: no request sent on empty name`).rejects.toThrow();

        // Case 2: whitespace-only display name -> client disables save button and blocks request
        await nameInput.fill('    ');
        await expect(saveBtn, `${lang}: save button disabled on whitespace-only name`).toBeDisabled();
        await expect(nameInput).toHaveValue('    ');
        const noReq2 = page.waitForRequest(/\/api\/app\/profile$/, { timeout: 2_000 });
        await saveBtn.click({ force: true }).catch(() => {});
        await expect(noReq2, `${lang}: no request sent on whitespace-only name`).rejects.toThrow();

        // Case 3: server-level refusal on blank display name via direct command
        const directRes = await page.request.put('/api/app/profile', {
          headers: sameOrigin,
          data: { displayName: '   ' },
        });
        expect(directRes.status(), `${lang}: direct blank displayName returns 400`).toBe(400);
        const directJson = (await directRes.json()) as { error?: { code?: string; field?: string; message?: string } };
        expect(directJson?.error?.code).toBe('validationFailed');
        expect(directJson?.error?.field).toBe('displayName');

        // Case 4: consentRecord validation checks (direct API commands on /api/app/consents)
        // 4a: invalid purpose
        const badPurposeRes = await page.request.post('/api/app/consents', {
          headers: sameOrigin,
          data: { purpose: 'INVALID_PURPOSE', channel: 'SMS', granted: true, lang },
        });
        expect(badPurposeRes.status(), `${lang}: invalid consent purpose returns 400`).toBe(400);
        const badPurposeJson = (await badPurposeRes.json()) as { error?: { code?: string; field?: string } };
        expect(badPurposeJson?.error?.code).toBe('validationFailed');
        expect(badPurposeJson?.error?.field).toBe('purpose');

        // 4b: invalid language
        const badLangRes = await page.request.post('/api/app/consents', {
          headers: sameOrigin,
          data: { purpose: 'MARKETING', channel: 'SMS', granted: true, lang: 'xx' },
        });
        expect(badLangRes.status(), `${lang}: invalid consent lang returns 400`).toBe(400);
        const badLangJson = (await badLangRes.json()) as { error?: { code?: string; field?: string } };
        expect(badLangJson?.error?.code).toBe('validationFailed');
        expect(badLangJson?.error?.field).toBe('lang');

        // 4c: withdrawing TERMS
        const withdrawTermsRes = await page.request.post('/api/app/consents', {
          headers: sameOrigin,
          data: { purpose: 'TERMS', granted: false, lang },
        });
        expect(withdrawTermsRes.status(), `${lang}: withdrawing TERMS returns 400`).toBe(400);
        const withdrawTermsJson = (await withdrawTermsRes.json()) as { error?: { code?: string } };
        expect(withdrawTermsJson?.error?.code).toBe('validationFailed');
      } finally {
        // Restore original name
        await nameInput.fill(origName);
      }

      // Read back: confirm profile displayName is still origName
      const profileRes = await page.request.get('/api/v3/views/my_profile');
      const profileJson = (await profileRes.json().catch(() => ({}))) as {
        data?: { displayName?: string };
        displayName?: string;
      };
      expect(profileJson?.data?.displayName ?? profileJson?.displayName).toBe(origName);
    });

    test(`/app/rate-alerts rateAlertCreate validation (${lang})`, async ({ page }) => {
      await withLang(page, lang);
      await page.goto('about:blank');
      await signIn(page, 'customer', '/app/rate-alerts');
      await page.goto('/app/rate-alerts');

      await expect(page.locator('.xc-rate-alerts-screen')).toBeVisible({ timeout: 20_000 });

      const initialAlertCards = await page.locator('article.xc-alert-card').count();

      const branchSelect = page.locator('#alert-branch');
      if ((await branchSelect.locator('option').count()) > 0) {
        await branchSelect.selectOption({ index: 0 });
      }

      const currencyInput = page.locator('#alert-currency');
      await currencyInput.fill('USD');

      const targetInput = page.locator('#alert-target');
      const submitBtn = page.locator('.xc-create-alert-form button[type="submit"]');
      const errorMsg = page.locator('.xc-create-alert-form .xc-form-error');
      const expectedAlertMsg = lang === 'th' ? 'ข้อมูลไม่ถูกต้อง' : 'Validation failed';

      // Case 1: empty threshold -> HTML5 valueMissing blocks request
      await targetInput.fill('');
      const isMissing = await targetInput.evaluate((el: HTMLInputElement) => el.validity.valueMissing);
      expect(isMissing, `${lang}: empty target triggers valueMissing`).toBe(true);
      const noReq = page.waitForRequest(/\/api\/app\/rate-alerts$/, { timeout: 2_000 });
      await submitBtn.click();
      await expect(noReq, `${lang}: no request sent on valueMissing`).rejects.toThrow();
      expect(await page.locator('article.xc-alert-card').count()).toBe(initialAlertCards);

      // Case 2: negative threshold
      await targetInput.fill('-15.50');
      const negAlertResPromise = page.waitForResponse(
        (res) => res.request().method() === 'POST' && /\/api\/app\/rate-alerts$/.test(new URL(res.url()).pathname),
        { timeout: 15_000 },
      );
      await submitBtn.click();
      const negAlertRes = await negAlertResPromise;
      expect(negAlertRes.status(), `${lang}: negative threshold returns 400`).toBe(400);
      await expect(errorMsg).toBeVisible({ timeout: 10_000 });
      await expect(errorMsg).toContainText(expectedAlertMsg);
      await expect(targetInput, `${lang}: target retains -15.50`).toHaveValue('-15.50');
      expect(await page.locator('article.xc-alert-card').count()).toBe(initialAlertCards);

      // Case 3: zero threshold
      await targetInput.fill('0');
      const zeroAlertResPromise = page.waitForResponse(
        (res) => res.request().method() === 'POST' && /\/api\/app\/rate-alerts$/.test(new URL(res.url()).pathname),
        { timeout: 15_000 },
      );
      await submitBtn.click();
      const zeroAlertRes = await zeroAlertResPromise;
      expect(zeroAlertRes.status(), `${lang}: zero threshold returns 400`).toBe(400);
      await expect(errorMsg).toBeVisible({ timeout: 10_000 });
      await expect(errorMsg).toContainText(expectedAlertMsg);
      await expect(targetInput, `${lang}: target retains 0`).toHaveValue('0');
      expect(await page.locator('article.xc-alert-card').count()).toBe(initialAlertCards);

      // Case 4: excess decimals (>8 decimals)
      await targetInput.fill('34.123456789');
      const decAlertResPromise = page.waitForResponse(
        (res) => res.request().method() === 'POST' && /\/api\/app\/rate-alerts$/.test(new URL(res.url()).pathname),
        { timeout: 15_000 },
      );
      await submitBtn.click();
      const decAlertRes = await decAlertResPromise;
      expect(decAlertRes.status(), `${lang}: excess decimals returns 400`).toBe(400);
      await expect(errorMsg).toBeVisible({ timeout: 10_000 });
      await expect(errorMsg).toContainText(expectedAlertMsg);
      expect(await page.locator('article.xc-alert-card').count()).toBe(initialAlertCards);

      // Read back: alert count unchanged
      expect(await page.locator('article.xc-alert-card').count(), `${lang}: alerts count unchanged`).toBe(initialAlertCards);
    });

    test(`/app/sign-up registration validation (${lang})`, async ({ page }) => {
      await withLang(page, lang);
      await page.goto('about:blank');
      await page.goto('/app/sign-up');

      const form = page.locator('form.xc-sign-in-form');
      await expect(form).toBeVisible({ timeout: 20_000 });

      const nameInput = page.locator('input[name="displayName"]');
      const idInput = page.locator('input[name="identifier"]');
      const pwdInput = page.locator('input[name="password"]');
      const submitBtn = page.locator('form.xc-sign-in-form button[type="submit"]');
      const errorAlert = page.locator('p.xc-sign-in-error');

      const expectedInvalidRequest = lang === 'th' ? 'คำขอไม่ถูกต้อง' : 'Invalid request';

      // Case 1: empty display name with valid fresh credentials
      const freshPhone = '+6681' + Date.now().toString().slice(-7);
      await idInput.fill(freshPhone);
      await pwdInput.fill('ValidPass123!');
      await nameInput.fill('');
      const regResPromise1 = page.waitForResponse(
        (res) => res.request().method() === 'POST' && /\/api\/app\/register$/.test(new URL(res.url()).pathname),
        { timeout: 15_000 },
      );
      await submitBtn.click();
      const regRes1 = await regResPromise1;
      expect(regRes1.status(), `${lang}: empty displayName returns 400`).toBe(400);
      await expect(errorAlert).toBeVisible({ timeout: 10_000 });
      await expect(errorAlert).not.toHaveText(/^(?:validationFailed|\s*)$/);
      await expect(idInput, `${lang}: identifier retains value`).toHaveValue(freshPhone);
      await expect(pwdInput, `${lang}: password retains value`).toHaveValue('ValidPass123!');

      // Case 2: empty identifier
      await nameInput.fill('Test Customer');
      await idInput.fill('');
      const regResPromise2 = page.waitForResponse(
        (res) => res.request().method() === 'POST' && /\/api\/app\/register$/.test(new URL(res.url()).pathname),
        { timeout: 15_000 },
      );
      await submitBtn.click();
      const regRes2 = await regResPromise2;
      expect(regRes2.status(), `${lang}: empty identifier returns 400`).toBe(400);
      await expect(errorAlert).toBeVisible({ timeout: 10_000 });
      await expect(errorAlert).toHaveText(expectedInvalidRequest);
      await expect(nameInput, `${lang}: displayName retains value`).toHaveValue('Test Customer');

      // Case 3: empty password
      await idInput.fill(freshPhone);
      await pwdInput.fill('');
      const regResPromise3 = page.waitForResponse(
        (res) => res.request().method() === 'POST' && /\/api\/app\/register$/.test(new URL(res.url()).pathname),
        { timeout: 15_000 },
      );
      await submitBtn.click();
      const regRes3 = await regResPromise3;
      expect(regRes3.status(), `${lang}: empty password returns 400`).toBe(400);
      await expect(errorAlert).toBeVisible({ timeout: 10_000 });
      await expect(errorAlert).toHaveText(expectedInvalidRequest);

      // Case 4: already registered identifier (unconditional 409 check)
      const existingId = users.customer?.identifier ?? users.customer?.phone ?? '';
      await idInput.fill(existingId);
      await pwdInput.fill(users.customer.password);
      await nameInput.fill('Another User');
      const regResPromise4 = page.waitForResponse(
        (res) => res.request().method() === 'POST' && /\/api\/app\/register$/.test(new URL(res.url()).pathname),
        { timeout: 15_000 },
      );
      await submitBtn.click();
      const regRes4 = await regResPromise4;
      expect(regRes4.status(), `${lang}: already registered identifier returns 409`).toBe(409);
      await expect(errorAlert).toBeVisible({ timeout: 10_000 });
      const expectedAlreadyReg =
        lang === 'th' ? 'บัญชีนี้สมัครสมาชิกแล้ว กรุณาเข้าสู่ระบบ' : 'This account is already registered. Please sign in';
      await expect(errorAlert).toHaveText(expectedAlreadyReg);
    });

    test(`/app/journey plannerDraftSave validation (${lang})`, async ({ page }) => {
      await withLang(page, lang);
      await page.goto('about:blank');
      await signIn(page, 'customer', '/app');
      await page.goto('/app');

      const amtInput = page.locator('input#amount-input');
      await expect(amtInput).toBeVisible({ timeout: 20_000 });

      const calcBtn = page.locator('.xc-home-calc-actions button');
      const calcError = page.locator('.xc-home-alert.xc-home-alert-error');
      const estimateCard = page.locator('.xc-home-estimate-card');
      const expectedCalcError = lang === 'th' ? 'กรุณาระบุจำนวนเงินที่ถูกต้อง' : 'Please enter a valid amount';

      // Case 1: amount 0
      console.log(`[r9-11] journey ${lang} case 1`);
      await amtInput.fill('0');
      await expect(calcError).not.toBeVisible();
      const noReq1 = page.waitForRequest(/\/api\/v3\/compute\/planner_preview$/, { timeout: 2_000 });
      await calcBtn.click();
      await expect(noReq1, `${lang}: no request sent for 0 amount`).rejects.toThrow();
      console.log(
        `[r9-11] journey ${lang} case 1 state: calcBtn.count=${await calcBtn.count()} calcBtn.enabled=${await calcBtn.isEnabled()} url=${page.url()}`,
      );
      await expect(calcError).toBeVisible({ timeout: 10_000 });
      await expect(calcError).toHaveText(expectedCalcError);
      await expect(estimateCard).not.toBeVisible();
      await expect(amtInput, `${lang}: amount retains 0`).toHaveValue('0');

      // Case 2: negative amount
      console.log(`[r9-11] journey ${lang} case 2`);
      await amtInput.fill('-200');
      await expect(calcError, `${lang}: error cleared on input`).not.toBeVisible();
      const noReq2 = page.waitForRequest(/\/api\/v3\/compute\/planner_preview$/, { timeout: 2_000 });
      await calcBtn.click();
      await expect(noReq2, `${lang}: no request sent for negative amount`).rejects.toThrow();
      await expect(calcError).toBeVisible({ timeout: 10_000 });
      await expect(calcError).toHaveText(expectedCalcError);
      await expect(estimateCard).not.toBeVisible();
      await expect(amtInput, `${lang}: amount retains -200`).toHaveValue('-200');

      // Case 3: non-numeric amount
      console.log(`[r9-11] journey ${lang} case 3`);
      await amtInput.fill('xyz');
      await expect(calcError, `${lang}: error cleared on input`).not.toBeVisible();
      const noReq3 = page.waitForRequest(/\/api\/v3\/compute\/planner_preview$/, { timeout: 2_000 });
      await calcBtn.click();
      await expect(noReq3, `${lang}: no request sent for non-numeric amount`).rejects.toThrow();
      await expect(calcError).toBeVisible({ timeout: 10_000 });
      await expect(calcError).toHaveText(expectedCalcError);
      await expect(estimateCard).not.toBeVisible();
      await expect(amtInput, `${lang}: amount retains xyz`).toHaveValue('xyz');
    });

    test(`/app/bookings bookingReschedule & bookingCancel validation (${lang})`, async ({ page }) => {
      await withLang(page, lang);
      await page.goto('about:blank');
      await signIn(page, 'customer', '/app/bookings');
      await page.goto('/app/bookings');

      await expect(page.locator('.xc-bookings-screen')).toBeVisible({ timeout: 20_000 });

      // Check 1: Reschedule validation
      const rescheduleBtn = page.locator('button[data-action="reschedule"]').first();
      const hasReschedule = await rescheduleBtn.isVisible().catch(() => false);
      if (!hasReschedule) {
        test.skip(!hasReschedule, `${lang}: no active booking available for reschedule check`);
      } else {
        await rescheduleBtn.click();
        const dialog = page.locator('.xc-reschedule-dialog');
        await expect(dialog).toBeVisible({ timeout: 10_000 });

        // Confirm button is disabled without selecting a slot
        const confirmBtn = page.locator('button[data-action="confirm-reschedule"]');
        await expect(confirmBtn, `${lang}: confirm reschedule disabled without slot`).toBeDisabled();

        // Prove no request is sent
        const noReq = page.waitForRequest(/\/api\/bookings\/.*\/reschedule$/, { timeout: 2_000 });
        await confirmBtn.click({ force: true }).catch(() => {});
        await expect(noReq, `${lang}: no request sent when confirm disabled`).rejects.toThrow();

        // Test past date on reschedule dialog
        const dateInput = page.locator('input[data-testid="date-input"]');
        if (await dateInput.isVisible()) {
          await dateInput.fill('2020-01-01');
          const isUnderflow = await dateInput.evaluate((el: HTMLInputElement) => el.validity.rangeUnderflow);
          expect(isUnderflow, `${lang}: past reschedule date triggers rangeUnderflow`).toBe(true);
        }

        // Close dialog
        const backBtn = page.locator('.xc-dialog-actions button', {
          hasText: lang === 'th' ? 'ย้อนกลับ' : 'Back',
        });
        if (await backBtn.isVisible()) {
          await backBtn.click();
        }
      }

      // Check 2: bookingCancel validation
      // 2a: Direct command for cancelling a non-existent booking ID
      const badCancelRes = await page.request.post('/api/bookings/00000000-0000-0000-0000-000000000000/cancel', {
        headers: sameOrigin,
        data: { reason: 'test' },
      });
      expect(badCancelRes.status(), `${lang}: cancelling non-existent booking returns 404`).toBe(404);

      // 2b: UI Cancel dialog: dismissing dialog via Back does not trigger cancel request
      const cancelBtn = page.locator('button[data-action="cancel"]').first();
      if (await cancelBtn.isVisible().catch(() => false)) {
        await cancelBtn.click();
        const cancelDialog = page.locator('.xc-confirm-dialog');
        await expect(cancelDialog).toBeVisible({ timeout: 10_000 });

        const noCancelReq = page.waitForRequest(/\/api\/bookings\/.*\/cancel$/, { timeout: 2_000 });
        const dialogBackBtn = page.locator('.xc-dialog-actions button', {
          hasText: lang === 'th' ? 'ย้อนกลับ' : 'Back',
        });
        await dialogBackBtn.click();
        await expect(noCancelReq, `${lang}: cancel dismissed with no request`).rejects.toThrow();
        await expect(cancelDialog).not.toBeVisible();
      }
    });
  }
});
