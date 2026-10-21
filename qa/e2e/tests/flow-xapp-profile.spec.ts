// QA-F14c — flow: customer profile and consent edit and restore (design docs/design/qa-e2e.md §4 row F14c; #4111).
// Import only from './helpers'. Never run against a host from a worker's machine (E-3): check with --list and tsc.
//
// Elements cited from web/src/screens/customer/profile/Profile.tsx:
// - [data-field="displayName"]         line 228
// - [data-field="preferredLanguage"]   line 240
// - [data-action="save-profile"]       line 255
// - [data-testid="profile-saved"]      line 213 (.xc-profile-saved)
// - [data-testid="profile-error"]      line 206 (.xc-profile-error)
// - input[data-consent="EMAIL"]        line 270 (checkbox under .xc-profile-consents)
import { expect, test } from '@playwright/test';
import { shot, signIn } from './helpers';

test.describe('QA-F14c flow: customer profile and consent edit and restore', () => {
  test('customer edits profile and consent and restores both', async ({ page }) => {
    // Navigate and sign in as customer
    await signIn(page, 'customer', '/app/profile');
    await expect(page.locator('.xc-profile-body')).toBeVisible({ timeout: 20_000 });

    const nameInput = page.locator('input[data-field="displayName"]');
    const langSelect = page.locator('select[data-field="preferredLanguage"]');
    const saveButton = page.locator('button[data-action="save-profile"]');
    const savedNotice = page.locator('[data-testid="profile-saved"]');
    const emailCheckbox = page.locator('input[data-consent="EMAIL"]');

    // Step 1: read the current name and language (kept for finally)
    await expect(nameInput).toBeVisible({ timeout: 15_000 });
    await expect(langSelect).toBeVisible({ timeout: 15_000 });

    const origName = await nameInput.inputValue();
    const origLang = await langSelect.inputValue();

    const targetLang = origLang === 'en' ? 'th' : 'en';
    const testName = `E2E-${Date.now()}`;

    // A restore that fails in a finally must turn the test red, not just log (the customer would stay changed).
    const restoreFailures: string[] = [];

    // Step 2 & 3: edit profile and restore in finally
    try {
      // Profile save is PUT /api/app/profile
      const putResponsePromise = page.waitForResponse(
        (res) => res.request().method() === 'PUT' && /\/api\/app\/profile$/.test(new URL(res.url()).pathname),
        { timeout: 15_000 },
      );

      await nameInput.fill(testName);
      await langSelect.selectOption(targetLang);
      await expect(saveButton).toBeEnabled();
      await saveButton.click();

      const putRes = await putResponsePromise;
      expect(putRes.ok(), `PUT /api/app/profile responded ok: ${putRes.status()}`).toBeTruthy();
      await expect(savedNotice).toBeVisible({ timeout: 15_000 });

      // Verify the values hold after a reload
      await page.reload();
      await expect(page.locator('.xc-profile-body')).toBeVisible({ timeout: 20_000 });
      await expect(nameInput).toHaveValue(testName);
      await expect(langSelect).toHaveValue(targetLang);

      await shot(page, 'app-b', 'profile-edited', targetLang);
    } finally {
      // Step 3: finally restore both
      try {
        const curName = await nameInput.inputValue().catch(() => null);
        const curLang = await langSelect.inputValue().catch(() => null);
        if (curName !== origName || curLang !== origLang) {
          const restorePutPromise = page.waitForResponse(
            (res) => res.request().method() === 'PUT' && /\/api\/app\/profile$/.test(new URL(res.url()).pathname),
            { timeout: 15_000 },
          );
          restorePutPromise.catch(() => {});
          await nameInput.fill(origName);
          await langSelect.selectOption(origLang);
          if (await saveButton.isEnabled()) {
            await saveButton.click();
            const restoreRes = await restorePutPromise;
            expect(restoreRes.ok(), `restore PUT /api/app/profile: ${restoreRes.status()}`).toBeTruthy();
            await expect(savedNotice).toBeVisible({ timeout: 15_000 });
          }
          await page.reload();
          await expect(page.locator('.xc-profile-body')).toBeVisible({ timeout: 20_000 });
          await expect(nameInput).toHaveValue(origName);
          await expect(langSelect).toHaveValue(origLang);
        }
      } catch (err) {
        console.error('Failed to restore customer profile in finally:', err);
        restoreFailures.push(`profile: ${String(err)}`);
      }
    }

    // Step 4 & 5: toggle data-consent=EMAIL, reload, and restore in finally
    await expect(emailCheckbox).toBeVisible({ timeout: 15_000 });
    const origConsent = await emailCheckbox.isChecked();

    try {
      const consentPostPromise = page.waitForResponse(
        (res) => res.request().method() === 'POST' && /\/api\/app\/consents$/.test(new URL(res.url()).pathname),
        { timeout: 15_000 },
      );

      await emailCheckbox.setChecked(!origConsent);
      const postRes = await consentPostPromise;
      if (postRes.status() === 409 && (await postRes.text()).includes('consentTextMissing')) {
        // No MARKETING consent text seeded for this language (consent_record.rs:126-133); the UI rolls the box back.
        const reason = 'no MARKETING consent text seeded (409 consentTextMissing)';
        console.log(`[F14c] skip: ${reason}`);
        test.skip(true, reason);
      }
      expect(postRes.ok(), `POST /api/app/consents responded ok: ${postRes.status()}`).toBeTruthy();

      // Check flipped after a reload
      await page.reload();
      await expect(page.locator('.xc-profile-body')).toBeVisible({ timeout: 20_000 });
      await expect(emailCheckbox).toBeChecked({ checked: !origConsent });
    } finally {
      // Step 5: finally toggle it back
      try {
        const curChecked = await emailCheckbox.isChecked().catch(() => null);
        if (curChecked !== null && curChecked !== origConsent) {
          const restoreConsentPromise = page.waitForResponse(
            (res) => res.request().method() === 'POST' && /\/api\/app\/consents$/.test(new URL(res.url()).pathname),
            { timeout: 15_000 },
          );
          await emailCheckbox.setChecked(origConsent);
          const restoreConsentRes = await restoreConsentPromise;
          expect(restoreConsentRes.ok(), `restore POST /api/app/consents: ${restoreConsentRes.status()}`).toBeTruthy();
          await page.reload();
          await expect(page.locator('.xc-profile-body')).toBeVisible({ timeout: 20_000 });
          await expect(emailCheckbox).toBeChecked({ checked: origConsent });
        }
      } catch (err) {
        console.error('Failed to restore customer consent in finally:', err);
        restoreFailures.push(`consent: ${String(err)}`);
      }
    }
    expect(restoreFailures, 'every change made by this test was restored').toEqual([]);
  });
});
