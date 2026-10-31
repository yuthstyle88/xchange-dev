// Shared desk exchange helper (quote -> customer -> booking -> payment -> receipt).
// Extracted from flow-exchange.spec.ts per #5795.
// Import only from './helpers'. Never run against a host from a worker's machine (E-3): check with --list and tsc.
//
// Selectors cited from web/src/screens/:
// - desk/steps/rate/step.tsx:62-94,104,136    select[name="type"], select[name="currency"], input[name="amount"], .desk-rate-line strong, submit Next (rate/i18n.ts:5,22)
// - desk/steps/rate/step.tsx:76               option[value="SELL"]
// - desk/steps/customer/step.tsx:159-177,256  button "ข้ามข้อมูลลูกค้า →" / "Skip customer details →" (customer/i18n.ts:18,44)
// - desk/steps/booking/step.tsx:54            button "ยืนยันการจอง →" / "Confirm booking →" (booking/i18n.ts:10,22)
// - desk/steps/payment/step.tsx:110,135,168,229,246 select[name="method"], [name="account"], input[name="paymentChecked"], window.confirm, submit "ยืนยันและบันทึกบัญชี →" / "Confirm and save →" (payment/i18n.ts:17,41)
// - desk/steps/receipt/step.tsx:21,32-34      section.desk-form strong (/^XC-\d{8}-\d{6}$/)
import { expect, type Page } from '@playwright/test';
import { ensureQueueOpen, posted, shot, users } from './helpers';

const EXCHANGE_AMOUNT = '1';

interface TransactionAnswer {
  id?: string;
  status?: string;
  documentRef?: string | null;
}

/**
 * Runs the desk exchange steps 2-5 up to the receipt (steps/rate -> customer -> booking -> payment -> receipt).
 * Returns the receipt documentRef number matching /^XC-\d{8}-\d{6}$/.
 */
export async function runDeskExchange(
  teller: Page,
  manager: Page,
  type: 'BUY' | 'SELL',
  onQueueOpened?: (opened: boolean) => void,
): Promise<string> {
  const currency = users.currency ?? 'USD';
  // Step 2: teller opens /desk?branch=<branch>, selects BUY/SELL + currency + amount, waits for POST /api/quotes, submits Next.
  await teller.goto('/desk?branch=' + users.branch);
  await expect(
    teller.locator(`select[name="currency"] option[value="${currency}"]`),
    `rate for ${currency} loaded in desk_config (rate/step.tsx:81)`,
  ).toHaveCount(1, { timeout: 20_000 });

  const quoteRes = await posted(teller, /^\/api\/quotes$/, async () => {
    await teller.locator('select[name="type"]').selectOption(type); // rate/step.tsx:75-76
    await teller.locator('select[name="currency"]').selectOption(currency); // rate/step.tsx:81
    await teller.locator('input[name="amount"]').fill(EXCHANGE_AMOUNT); // rate/step.tsx:94
  });
  console.log(`[F6] POST /api/quotes (${type}) ${quoteRes.text}`);
  expect(quoteRes.ok, `POST /api/quotes (${type}): ${quoteRes.text}`).toBeTruthy();

  await expect(
    teller.locator('.desk-rate-line strong'),
    `quoted rate displayed after POST /api/quotes (${quoteRes.text}) (rate/step.tsx:104)`,
  ).toBeVisible({ timeout: 15_000 });
  await shot(teller, 'desk', `exchange-${type.toLowerCase()}-1-rate`);

  await teller.getByRole('button', { name: /ถัดไป · พาสปอร์ต \/ ลูกค้า|Next · Passport \/ customer/ }).click(); // rate/step.tsx:136

  const skipBtn = teller.getByRole('button', { name: /ข้ามข้อมูลลูกค้า|Skip customer details/ });
  await expect(skipBtn, 'customer step shows (steps/customer/step.tsx:256)').toBeVisible({ timeout: 15_000 });

  // Step 3: customer step — "Skip customer details" auto-fills WALK-IN + placeholder image and reserves the customer.
  const reserveRes = await posted(teller, /^\/api\/rpc$/, () => skipBtn.click());
  console.log(`[F6] POST /api/rpc (reserveCustomer, ${type}) ${reserveRes.text}`);
  expect(reserveRes.ok, `reserveCustomer on skip: ${reserveRes.text}`).toBeTruthy();

  const confirmBookingBtn = teller.getByRole('button', { name: /ยืนยันการจอง|Confirm booking/ });
  await expect(confirmBookingBtn, 'booking step shows (steps/booking/step.tsx:54)').toBeVisible({
    timeout: 15_000,
  });
  await shot(teller, 'desk', `exchange-${type.toLowerCase()}-2-booking`);

  const opened = await ensureQueueOpen(manager);
  if (opened && onQueueOpened) onQueueOpened(true);

  // Step 4: booking step — "Confirm booking" advances to the payment step.
  await confirmBookingBtn.click();

  const methodSelect = teller.locator('select[name="method"]');
  await expect(methodSelect, 'payment step shows (steps/payment/step.tsx:135)').toBeVisible({ timeout: 15_000 });

  // Step 5: payment step — method Cash, account = first offered option, tick paymentChecked, accept window.confirm, submit.
  await methodSelect.selectOption('Cash');
  const accountField = teller.locator('[name="account"]');
  await expect(accountField, 'payment account control (steps/payment/step.tsx:168,187)').toBeVisible({
    timeout: 15_000,
  });
  const accountOptions = accountField.locator('option');
  if ((await accountOptions.count()) > 0) {
    const firstOption = (await accountOptions.first().getAttribute('value')) ?? '';
    await accountField.selectOption(firstOption);
  } else if ((await accountField.inputValue()).trim() === '') {
    await accountField.fill('Cash');
  }

  await teller.locator('input[name="paymentChecked"]').check(); // payment/step.tsx:260
  await shot(teller, 'desk', `exchange-${type.toLowerCase()}-3-payment`);

  teller.once('dialog', (d) => d.accept());
  const createRes = await posted(teller, /^\/api\/transactions$/, () =>
    teller.getByRole('button', { name: /ยืนยันและบันทึกบัญชี|Confirm and save/ }).click(), // payment/step.tsx:280
  );
  console.log(`[F6] POST /api/transactions (${type}) ${createRes.text}`);
  expect(createRes.ok, `POST /api/transactions (${type}): ${createRes.text}`).toBeTruthy();
  const createdTx = JSON.parse(createRes.body) as TransactionAnswer;
  const txId = String(createdTx.id ?? '');
  expect(txId, `transaction id from POST /api/transactions (${type}): ${createRes.text}`).not.toBe('');

  const confirmRes = await posted(
    teller,
    new RegExp(`^/api/transactions/${encodeURIComponent(txId)}/confirm$`),
    async () => {},
  );
  console.log(`[F6] POST /api/transactions/${txId}/confirm (${type}) ${confirmRes.text}`);
  expect(confirmRes.ok, `POST /api/transactions/${txId}/confirm (${type}): ${confirmRes.text}`).toBeTruthy();
  const confirmedTx = JSON.parse(confirmRes.body) as TransactionAnswer;
  expect(
    confirmedTx.status,
    `POST /api/transactions/${txId}/confirm status (${type}): ${confirmRes.text}`,
  ).toBe('CONFIRMED');

  const completeRes = await posted(
    teller,
    new RegExp(`^/api/transactions/${encodeURIComponent(txId)}/complete$`),
    async () => {},
  );
  console.log(`[F6] POST /api/transactions/${txId}/complete (${type}) ${completeRes.text}`);
  expect(completeRes.ok, `POST /api/transactions/${txId}/complete (${type}): ${completeRes.text}`).toBeTruthy();
  const completedTx = JSON.parse(completeRes.body) as TransactionAnswer;
  expect(
    completedTx.status,
    `POST /api/transactions/${txId}/complete status (${type}): ${completeRes.text}`,
  ).toBe('COMPLETED');

  // Step 6: receipt step — <strong> inside section.desk-form matches /^XC-\d{8}-\d{6}$/ (steps/receipt/step.tsx:32-34).
  const receiptRef = teller.locator('section.desk-form strong').first();
  await expect(
    receiptRef,
    `receipt documentRef after complete (${completeRes.text}) (steps/receipt/step.tsx:32-34,72)`,
  ).toHaveText(/^XC-\d{8}-\d{6}$/, { timeout: 20_000 });
  const receiptDocRef = (await receiptRef.innerText()).trim();
  await shot(teller, 'desk', `exchange-${type.toLowerCase()}-4-receipt`);
  return receiptDocRef;
}
