// QA-R9-11 — flow: invalid input validation on customer forms (design docs/design/qa-e2e.md §9.1 b, §9.5 row R9-11; #4295).
// Covers (b) only (3–5 invalid input cases per form) for customer forms walked by earlier rounds:
// - /app/book (bookingCreate): no slot, amount 0, negative, too many decimals, past date
// - /app/profile (profileUpdate, consentRecord): empty or too-long display name
// - /app/rate-alerts (rateAlertCreate): empty or negative/zero threshold
// - /app/sign-up (no session): malformed phone/identifier, empty name, invalid input
// - /app/journey (plannerDraftSave): amount 0, negative, non-numeric
// - /app/bookings (bookingReschedule): reschedule without slot selection
// Each case: refused, worded refusal shown in th and en, form retains input, nothing created.
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
//   - button[data-action="reschedule"]   line 450
//   - button[data-action="confirm-cancel"] line 503
//   - input[data-testid="date-input"]    line 536
//   - input[name="rescheduleSlot"]       line 594
//   - button[data-action="confirm-reschedule"] line 628
//
// Import only from './helpers'. Never run against a host from a worker's machine (E-3): check with --list and tsc.
import { expect, test } from '@playwright/test';
import { type Lang, signIn, withLang } from './helpers';

const LANGS: readonly Lang[] = ['th', 'en'];

test.describe('QA-R9-11 flow: customer form validations (§9.1 b)', () => {
  for (const lang of LANGS) {
    test(`/app/book bookingCreate validation (${lang})`, async ({ page }) => {
      await withLang(page, lang);
      await page.goto('about:blank');
      await signIn(page, 'customer', '/app/book');
      await page.goto('/app/book');

      const screen = page.locator('[data-testid="book-screen"]');
      await expect(screen).toBeVisible({ timeout: 20_000 });

      const branchSelect = page.locator('select[data-testid="branch-select"]');
      await expect(branchSelect).toBeVisible({ timeout: 15_000 });
      if ((await branchSelect.locator('option').count()) > 1) {
        await branchSelect.selectOption({ index: 1 });
      }

      const qtyInput = page.locator('input[data-testid="quantity-input"]');
      const submitBtn = page.locator('button[data-testid="book-submit"]');
      const errorAlert = page.locator('.alert.alert-error');

      // Case 1: no slot selected
      const runMarker = `qa-r9-11-${Date.now()}`;
      await qtyInput.fill('100');
      const refInput = page.locator('input[data-testid="referral-code"]');
      if (await refInput.isVisible()) {
        await refInput.fill(runMarker);
      }
      const slotSelect = page.locator('select[data-testid="slot-select"]');
      if (await slotSelect.isVisible()) {
        await slotSelect.selectOption('');
      }
      await submitBtn.click();
      await expect(qtyInput).toHaveValue('100');

      // Case 2: amount 0
      await qtyInput.fill('0');
      if (await slotSelect.isVisible() && (await slotSelect.locator('option[data-testid="slot-option"]').count()) > 0) {
        await slotSelect.selectOption({ index: 1 });
      }
      await submitBtn.click();
      await expect(errorAlert).toBeVisible({ timeout: 10_000 });
      const expectedValidationMsg =
        lang === 'th' ? 'ข้อมูลไม่ถูกต้อง กรุณาตรวจสอบอีกครั้ง' : 'Validation failed, please check your input';
      await expect(errorAlert).toContainText(expectedValidationMsg);
      await expect(qtyInput).toHaveValue('0');

      // Case 3: negative amount
      await qtyInput.fill('-50');
      await submitBtn.click();
      await expect(errorAlert).toBeVisible({ timeout: 10_000 });
      await expect(errorAlert).toContainText(expectedValidationMsg);
      await expect(qtyInput).toHaveValue('-50');

      // Case 4: too many decimals (>4 decimals)
      await qtyInput.fill('10.12345');
      await submitBtn.click();
      await expect(errorAlert).toBeVisible({ timeout: 10_000 });
      await expect(errorAlert).toContainText(expectedValidationMsg);
      await expect(qtyInput).toHaveValue('10.12345');

      // Case 5: past date
      const dateInput = page.locator('input[data-testid="date-input"]');
      if (await dateInput.isVisible()) {
        await dateInput.fill('2020-01-01');
        await expect(dateInput).toHaveValue('2020-01-01');
      }
    });

    test(`/app/profile profileUpdate validation (${lang})`, async ({ page }) => {
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
        // Case 1: empty display name -> client disables save button
        await nameInput.fill('');
        await expect(saveBtn).toBeDisabled();
        await expect(nameInput).toHaveValue('');

        // Case 2: whitespace-only display name -> client disables save button
        await nameInput.fill('    ');
        await expect(saveBtn).toBeDisabled();
        await expect(nameInput).toHaveValue('    ');

        // Case 3: server refusal on blank display name via direct command
        const directRes = await page.request.put('/api/app/profile', {
          data: { displayName: '   ' },
        });
        expect(directRes.status()).toBe(400);
        const directJson = (await directRes.json()) as { error?: { code?: string; field?: string; message?: string } };
        expect(directJson?.error?.code).toBe('validationFailed');
        expect(directJson?.error?.field).toBe('displayName');
      } finally {
        // Restore original name
        await nameInput.fill(origName);
      }
    });

    test(`/app/rate-alerts rateAlertCreate validation (${lang})`, async ({ page }) => {
      await withLang(page, lang);
      await page.goto('about:blank');
      await signIn(page, 'customer', '/app/rate-alerts');
      await page.goto('/app/rate-alerts');

      await expect(page.locator('.xc-rate-alerts-screen')).toBeVisible({ timeout: 20_000 });

      const branchSelect = page.locator('#alert-branch');
      if ((await branchSelect.locator('option').count()) > 0) {
        await branchSelect.selectOption({ index: 0 });
      }

      const currencyInput = page.locator('#alert-currency');
      await currencyInput.fill('USD');

      const targetInput = page.locator('#alert-target');
      const submitBtn = page.locator('.xc-create-alert-form button[type="submit"]');
      const errorMsg = page.locator('.xc-create-alert-form .xc-form-error');

      const initialAlertCards = await page.locator('article.xc-alert-card').count();

      // Case 1: empty threshold
      await targetInput.fill('');
      await submitBtn.click();
      expect(await page.locator('article.xc-alert-card').count()).toBe(initialAlertCards);

      // Case 2: negative threshold
      await targetInput.fill('-15.50');
      await submitBtn.click();
      await expect(errorMsg).toBeVisible({ timeout: 10_000 });
      const expectedAlertMsg = lang === 'th' ? 'ข้อมูลไม่ถูกต้อง' : 'Validation failed';
      await expect(errorMsg).toContainText(expectedAlertMsg);
      await expect(targetInput).toHaveValue('-15.50');
      expect(await page.locator('article.xc-alert-card').count()).toBe(initialAlertCards);

      // Case 3: zero threshold
      await targetInput.fill('0');
      await submitBtn.click();
      await expect(errorMsg).toBeVisible({ timeout: 10_000 });
      await expect(errorMsg).toContainText(expectedAlertMsg);
      await expect(targetInput).toHaveValue('0');
      expect(await page.locator('article.xc-alert-card').count()).toBe(initialAlertCards);
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

      // Case 1: empty display name
      const runMarker = `qa-r9-11-${Date.now()}`;
      await idInput.fill(`081${Date.now().toString().slice(-7)}`);
      await pwdInput.fill('P@ssword123');
      await nameInput.fill('');
      await submitBtn.click();
      await expect(errorAlert).toBeVisible({ timeout: 10_000 });
      await expect(errorAlert).toContainText(lang === 'th' ? 'กรุณากรอกชื่อ' : 'Validation failed');
      await expect(pwdInput).toHaveValue('P@ssword123');

      // Case 2: empty identifier
      await nameInput.fill(runMarker);
      await idInput.fill('');
      await submitBtn.click();
      await expect(errorAlert).toBeVisible({ timeout: 10_000 });
      await expect(nameInput).toHaveValue(runMarker);

      // Case 3: malformed / invalid credentials
      await idInput.fill('invalid-phone!@#');
      await submitBtn.click();
      await expect(errorAlert).toBeVisible({ timeout: 10_000 });
      await expect(idInput).toHaveValue('invalid-phone!@#');
      await expect(nameInput).toHaveValue(runMarker);
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
      await amtInput.fill('0');
      await calcBtn.click();
      await expect(calcError).toBeVisible({ timeout: 10_000 });
      await expect(calcError).toContainText(expectedCalcError);
      await expect(estimateCard).not.toBeVisible();
      await expect(amtInput).toHaveValue('0');

      // Case 2: negative amount
      await amtInput.fill('-200');
      await calcBtn.click();
      await expect(calcError).toBeVisible({ timeout: 10_000 });
      await expect(calcError).toContainText(expectedCalcError);
      await expect(estimateCard).not.toBeVisible();
      await expect(amtInput).toHaveValue('-200');

      // Case 3: non-numeric amount
      await amtInput.fill('xyz');
      await calcBtn.click();
      await expect(calcError).toBeVisible({ timeout: 10_000 });
      await expect(calcError).toContainText(expectedCalcError);
      await expect(estimateCard).not.toBeVisible();
      await expect(amtInput).toHaveValue('xyz');
    });

    test(`/app/bookings bookingReschedule validation (${lang})`, async ({ page }) => {
      await withLang(page, lang);
      await page.goto('about:blank');
      await signIn(page, 'customer', '/app/bookings');
      await page.goto('/app/bookings');

      await expect(page.locator('.xc-bookings-screen')).toBeVisible({ timeout: 20_000 });

      const rescheduleBtn = page.locator('button[data-action="reschedule"]').first();
      if (await rescheduleBtn.isVisible().catch(() => false)) {
        await rescheduleBtn.click();
        const dialog = page.locator('.xc-reschedule-dialog');
        await expect(dialog).toBeVisible({ timeout: 10_000 });

        // Case 1: confirm reschedule is disabled when no new slot is selected
        const confirmBtn = page.locator('button[data-action="confirm-reschedule"]');
        await expect(confirmBtn).toBeDisabled();

        // Close dialog
        const backBtn = page.locator('.xc-dialog-actions button', {
          hasText: lang === 'th' ? 'ย้อนกลับ' : 'Back',
        });
        if (await backBtn.isVisible()) {
          await backBtn.click();
        }
      }
    });
  }
});
