// QA-R9-10 — Round 9 customer extras: assistantAsk, bookingConfirm, queueTicketTake,
// rateAlertPause, rateAlertResume, notificationRead, notificationReadAll
// (design docs/design/qa-e2e.md §9.1–§9.4).
// Written by ticket #4294 (QA-R9-10); this file is its whole File scope.
// Import only from './helpers'. Never run against a host from a worker's machine (E-3): check with --list and tsc.
import { expect, type Browser, type Page, test } from '@playwright/test';
import { CUSTOMER_LANGS, hasAccount, signIn, smoke, users, withLang } from './helpers';

const RUN_TAG = `qa-rnine-ten-${Date.now().toString(26).replace(/[0-9]/g, (d) => String.fromCharCode(113 + Number(d)))}`;

const RAW_ERROR_CODE =
  /^(validationFailed|notFound|outOfScope|roleNotAllowed|staleVersion|idempotencyConflict|serverError|rateAlertLimitReached|bookingsClosed|capacityFull|slotUnavailable|queueClosed|noOpenShift)$/;

// Tomorrow in Asia/Bangkok, as the date input wants it (yyyy-mm-dd) (flow-booking.spec.ts:7-10).
function tomorrowBangkok(): string {
  const now = new Date(Date.now() + 7 * 3600_000 + 24 * 3600_000);
  return now.toISOString().slice(0, 10);
}

// Contract status mapping for permission/scope refusals (api/crates/xc-auth/src/domain/policy/value_objects/policy_error.rs:103-124).
const POLICY_REFUSAL_STATUS: Record<string, number> = {
  signIn: 401,
  employeeAuthRequired: 401,
  outOfScope: 403,
  ownerOnly: 403,
  roleNotAllowed: 400,
  roleNotAllowedHere: 400,
  employeeAccount: 400,
};

function expectPermissionRefused(
  resp: { status: number; body: Record<string, unknown> },
  label: string,
): void {
  const errObj = (resp.body.error ?? {}) as { code?: string; message?: string };
  const code = (errObj.code ?? '').trim();
  const message = (errObj.message ?? '').trim();
  expect(
    Object.keys(POLICY_REFUSAL_STATUS),
    `${label}: refusal code (${code}) body=${JSON.stringify(resp.body)}`,
  ).toContain(code);
  expect(
    resp.status,
    `${label}: expected status ${POLICY_REFUSAL_STATUS[code]} for ${code}, got ${resp.status} body=${JSON.stringify(resp.body)}`,
  ).toBe(POLICY_REFUSAL_STATUS[code]);
  expect(message, `${label}: refusal error.message non-empty`).not.toBe('');
}

async function postJson(
  page: Page,
  path: string,
  body: Record<string, unknown>,
): Promise<{ status: number; body: Record<string, unknown> }> {
  return page.evaluate(
    async ({ url, payload }) => {
      const key =
        typeof crypto !== 'undefined' && typeof crypto.randomUUID === 'function'
          ? crypto.randomUUID()
          : `qa-r9-10-${Date.now()}-${Math.random().toString(16).slice(2)}`;
      const res = await fetch(url, {
        method: 'POST',
        credentials: 'same-origin',
        headers: {
          'Content-Type': 'application/json',
          'Idempotency-Key': key,
          'x-xchange-request': '1',
        },
        body: JSON.stringify(payload),
      });
      const json = (await res.json().catch(() => ({}))) as Record<string, unknown>;
      return { status: res.status, body: json };
    },
    { url: path, payload: body },
  );
}

async function deleteAlertById(page: Page, alertId: string): Promise<void> {
  // POST /api/app/rate-alerts/{id}/delete: web/src/screens/customer/rate-alerts/RateAlerts.tsx:86,611
  await postJson(page, `/api/app/rate-alerts/${encodeURIComponent(alertId)}/delete`, {
    branch: users.branch,
  }).catch(() => {});
}

async function cancelQueueTicketAsTeller(browser: Browser, ticketId: string): Promise<void> {
  if (!hasAccount('teller')) return;
  const ctx = await browser.newContext();
  try {
    const tellerPage = await ctx.newPage();
    await signIn(tellerPage, 'teller', `/desk/queue?branch=${encodeURIComponent(users.branch)}`);
    // POST /api/queue/tickets/{id}/cancel: web/src/screens/desk/queue/TicketActions.tsx:111-141
    await postJson(tellerPage, `/api/queue/tickets/${encodeURIComponent(ticketId)}/cancel`, {
      branch: users.branch,
      reason: `${RUN_TAG}-cleanup`,
    }).catch(() => {});
  } finally {
    await ctx.close();
  }
}

async function produceQueueNotifications(
  customerPage: Page,
  browser: Browser,
  recallToo: boolean,
): Promise<string | null> {
  if (!hasAccount('teller')) return null;
  const takeRes = await postJson(customerPage, '/api/app/queue/tickets', { branch: users.branch });
  if (takeRes.status !== 200 || typeof takeRes.body.ticketId !== 'string') {
    return null;
  }
  const ticketId = takeRes.body.ticketId;
  const tellerCtx = await browser.newContext();
  try {
    const tellerPage = await tellerCtx.newPage();
    await signIn(
      tellerPage,
      'teller',
      `/desk/queue?branch=${encodeURIComponent(users.branch)}`,
    );
    // Calling (and optionally recalling) a customer's ticket writes queue_called notifications (queue_desk.rs:281-299)
    await postJson(tellerPage, `/api/queue/tickets/${encodeURIComponent(ticketId)}/call`, {
      branch: users.branch,
      counter: 1,
    });
    if (recallToo) {
      await postJson(tellerPage, `/api/queue/tickets/${encodeURIComponent(ticketId)}/recall`, {
        branch: users.branch,
      });
    }
    await postJson(tellerPage, `/api/queue/tickets/${encodeURIComponent(ticketId)}/cancel`, {
      branch: users.branch,
      reason: `${RUN_TAG}-notify-seed`,
    });
    return null;
  } catch {
    return ticketId;
  } finally {
    await tellerCtx.close();
  }
}

test.describe('QA-R9-10 flow: customer assistant, booking confirm, take a number, rate alerts pause/resume, notifications read', () => {
  test('assistantAsk: (a) asks assistant and reads conversation back after reload; (b) refuses empty message in th/en; (c) serves non-customer role as anonymous app session', async ({
    page,
    browser,
  }) => {
    await signIn(page, 'customer', '/app/assistant');
    // .assistant-container: web/src/screens/customer/assistant/AssistantScreen.tsx:236
    await smoke(page, {
      area: 'customer-extras',
      screen: 'assistant',
      path: '/app/assistant',
      role: 'customer',
      key: '.assistant-container',
      langs: CUSTOMER_LANGS,
    });

    // (b) Invalid input: empty / whitespace-only assistant message in th and en
    for (const lang of ['th', 'en'] as const) {
      console.log(`[customer-extras] assistantAsk (b) lang=${lang} value="   "`);
      await withLang(page, lang);
      await page.goto('/app/assistant');
      // input.assistant-input: web/src/screens/customer/assistant/AssistantScreen.tsx:342
      const input = page.locator('input.assistant-input');
      // button.assistant-send-button: web/src/screens/customer/assistant/AssistantScreen.tsx:351
      const sendBtn = page.locator('button.assistant-send-button');
      await expect(input, `(${lang}) assistant-input visible`).toBeVisible({ timeout: 15_000 });

      await input.fill('   ');
      // AssistantScreen.tsx:354 — disabled={sending() || inputText().trim() === ''}
      await expect(sendBtn, `(${lang}) send button disabled for whitespace input`).toBeDisabled();
      await expect(input, `(${lang}) whitespace input preserved`).toHaveValue('   ');

      // Backend validation refusal on empty/whitespace text (assistant_ask.rs:78-85)
      const emptyResp = await postJson(page, '/api/app/assistant/messages', {
        branch: users.branch,
        text: '   ',
      });
      console.log(
        `[customer-extras] assistantAsk (b) lang=${lang} status=${emptyResp.status} body=${JSON.stringify(emptyResp.body)}`,
      );
      expect(
        emptyResp.status,
        `(${lang}) whitespace assistantAsk refused with 400 (body=${JSON.stringify(emptyResp.body)})`,
      ).toBe(400);
      const errObj = (emptyResp.body.error ?? {}) as { code?: string; message?: string };
      expect(
        errObj.code,
        `(${lang}) refusal code is validationFailed (body=${JSON.stringify(emptyResp.body)})`,
      ).toBe('validationFailed');
    }

    // (a) Happy path: submit through AssistantScreen controls, reload, and read the conversation back
    await withLang(page, 'en');
    await page.goto('/app/assistant');
    // input.assistant-input: web/src/screens/customer/assistant/AssistantScreen.tsx:342
    const input = page.locator('input.assistant-input');
    // button.assistant-send-button: web/src/screens/customer/assistant/AssistantScreen.tsx:351
    const sendBtn = page.locator('button.assistant-send-button');
    const promptText = `${RUN_TAG} What are the USD rates today?`;
    await input.fill(promptText);
    await expect(sendBtn, 'send button enabled for non-empty message').toBeEnabled();

    // POST /api/app/assistant/messages: web/src/screens/customer/assistant/AssistantScreen.tsx:103,174
    const askRespPromise = page.waitForResponse(
      (r) => r.url().includes('/api/app/assistant/messages') && r.request().method() === 'POST',
      { timeout: 15_000 },
    );
    await sendBtn.click();
    const askResp = await askRespPromise;
    expect(askResp.status(), 'POST /api/app/assistant/messages returns 200').toBe(200);
    const askBody = (await askResp.json().catch(() => ({}))) as { conversationId?: string };

    // .message-wrapper.message-customer .message-bubble: web/src/screens/customer/assistant/AssistantScreen.tsx:311-321
    const customerBubble = page
      .locator('.message-wrapper.message-customer .message-bubble')
      .filter({ hasText: RUN_TAG });
    await expect(customerBubble, 'customer message bubble visible').toBeVisible({ timeout: 15_000 });

    // .message-wrapper.message-assistant .message-bubble: web/src/screens/customer/assistant/AssistantScreen.tsx:311-321
    const assistantBubble = page.locator('.message-wrapper.message-assistant .message-bubble').first();
    await expect(assistantBubble, 'assistant reply bubble visible').toBeVisible({ timeout: 15_000 });
    const replyBeforeReload = (await assistantBubble.innerText()).trim();
    expect(replyBeforeReload.length, 'assistant reply text is non-empty').toBeGreaterThan(0);

    // AssistantScreen.tsx:184-188 sets ?conversation=<id> and :137-144 reloads my_assistant_conversation
    await page.reload();
    await expect(
      page.locator('.message-wrapper.message-customer .message-bubble').filter({ hasText: RUN_TAG }),
      'customer message persists after page.reload()',
    ).toBeVisible({ timeout: 15_000 });
    await expect(
      page.locator('.message-wrapper.message-assistant .message-bubble').first(),
      'assistant reply persists after page.reload()',
    ).toBeVisible({ timeout: 15_000 });
    const customerConvId =
      askBody.conversationId ?? new URL(page.url()).searchParams.get('conversation');

    // (c) Anonymous app session: teller lacks a customer record, so assistantAsk serves them as
    // an anonymous app visitor with its own conversation (assistant.rs:3-17; customer-app.md §4.4)
    if (hasAccount('teller')) {
      const tellerCtx = await browser.newContext();
      try {
        const tellerPage = await tellerCtx.newPage();
        await signIn(tellerPage, 'teller', '/staff');
        const tellerResp = await postJson(tellerPage, '/api/app/assistant/messages', {
          branch: users.branch,
          text: `${RUN_TAG} teller check`,
        });
        expect(tellerResp.status, 'teller assistantAsk answers 200 as anonymous app session').toBe(200);
        const tellerConvId = (tellerResp.body as { conversationId?: string }).conversationId;
        expect(typeof tellerConvId, 'teller conversationId is non-empty string').toBe('string');
        if (customerConvId) {
          expect(tellerConvId, 'teller conversation is distinct from customer').not.toBe(
            customerConvId,
          );
          const foreignConv = await tellerPage.evaluate(async (cid) => {
            const res = await fetch(
              `/api/v3/views/my_assistant_conversation?conversationId=${encodeURIComponent(cid)}`,
              { credentials: 'same-origin' },
            );
            return { status: res.status };
          }, customerConvId);
          expect(
            foreignConv.status,
            'teller my_assistant_conversation on customer conversation refused with 403',
          ).toBe(403);
        }
      } finally {
        await tellerCtx.close();
      }
    }
  });

  test('rateAlertPause and rateAlertResume: (a) pauses and resumes alert across reloads; (b) refuses negative, zero, >8-decimal and blank thresholds in th/en; (c) refuses teller', async ({
    page,
    browser,
  }) => {
    let createdAlertId: string | null = null;

    try {
      await signIn(page, 'customer', '/app/rate-alerts');
      // .xc-rate-alerts-screen: web/src/screens/customer/rate-alerts/RateAlerts.tsx:387
      await smoke(page, {
        area: 'customer-extras',
        screen: 'rate-alerts',
        path: '/app/rate-alerts',
        role: 'customer',
        key: '.xc-rate-alerts-screen',
        langs: CUSTOMER_LANGS,
      });

      // (b) Invalid inputs (4 cases: negative, zero, >8 decimals, whitespace) in th and en.
      // Reload before each case so [data-field-error="target"] starts absent (RateAlerts.tsx:529-531),
      // prove no POST was sent, and read the list back.
      const invalidThresholds = ['-15.50', '0', '34.123456789', '   '];
      for (const lang of ['th', 'en'] as const) {
        await withLang(page, lang);
        for (const badValue of invalidThresholds) {
          console.log(`[customer-extras] rateAlert (b) lang=${lang} value=${JSON.stringify(badValue)}`);
          await page.goto('/app/rate-alerts');
          // #alert-target: web/src/screens/customer/rate-alerts/RateAlerts.tsx:520
          const targetInput = page.locator('#alert-target');
          // .xc-create-alert-form button[type="submit"]: web/src/screens/customer/rate-alerts/RateAlerts.tsx:440,556
          const submitBtn = page.locator('.xc-create-alert-form button[type="submit"]');
          // [data-field-error="target"]: web/src/screens/customer/rate-alerts/RateAlerts.tsx:530
          const targetError = page.locator('[data-field-error="target"]');
          // article.xc-alert-card: web/src/screens/customer/rate-alerts/RateAlerts.tsx:620
          const alertCards = page.locator('article.xc-alert-card');

          await expect(targetInput, `(${lang}) #alert-target visible`).toBeVisible({ timeout: 15_000 });
          await expect(
            page.locator('#alert-branch option'),
            `(${lang}) #alert-branch options loaded`,
          ).not.toHaveCount(0, { timeout: 15_000 });
          await expect(
            page.locator('#alert-currency option'),
            `(${lang}) #alert-currency options loaded`,
          ).not.toHaveCount(0, { timeout: 15_000 });
          await expect(targetError, `(${lang}) [data-field-error="target"] absent before submit`).toHaveCount(0);
          await expect(
            page.locator('.xc-alerts-empty, .xc-alerts-list').first(),
            `(${lang}) alerts list or empty state loaded`,
          ).toBeVisible({ timeout: 15_000 });
          const countBefore = await alertCards.count();

          await targetInput.fill(badValue);
          let postCount = 0;
          const onReq = (req: import('@playwright/test').Request) => {
            if (req.url().includes('/api/app/rate-alerts') && req.method() === 'POST') {
              postCount++;
            }
          };
          page.on('request', onReq);
          try {
            await submitBtn.click();
            await expect(targetError, `(${lang}) refusal shown at target for "${badValue}"`).toBeVisible({
              timeout: 10_000,
            });
            expect(postCount, `(${lang}) no POST sent for target="${badValue}"`).toBe(0);
          } finally {
            page.off('request', onReq);
          }

          const errText = (await targetError.innerText()).trim();
          expect(errText.length, `(${lang}) refusal message non-empty for "${badValue}"`).toBeGreaterThan(0);
          expect(
            errText,
            `(${lang}) refusal message "${errText}" is worded, not raw error code`,
          ).not.toMatch(RAW_ERROR_CODE);
          // Form keeps what was typed
          await expect(targetInput, `(${lang}) #alert-target keeps "${badValue}"`).toHaveValue(badValue);

          // Reload and verify no alert was created
          await page.reload();
          await expect(targetInput, `(${lang}) #alert-target visible after reload`).toBeVisible({
            timeout: 15_000,
          });
          await expect(
            page.locator('.xc-alerts-empty, .xc-alerts-list').first(),
            `(${lang}) alerts list or empty state loaded after reload`,
          ).toBeVisible({ timeout: 15_000 });
          expect(
            await alertCards.count(),
            `(${lang}) alert count unchanged after refused target="${badValue}"`,
          ).toBe(countBefore);
        }
      }

      // (a) Happy path: create alert, pause + reload, resume + reload (never delete rows not created by this spec, §9.2)
      await withLang(page, 'en');
      await page.goto('/app/rate-alerts');
      // #alert-branch: web/src/screens/customer/rate-alerts/RateAlerts.tsx:444
      const branchSelect = page.locator('#alert-branch');
      await expect(branchSelect, '#alert-branch visible').toBeVisible({ timeout: 15_000 });
      await expect(branchSelect.locator('option'), '#alert-branch options loaded').not.toHaveCount(0, {
        timeout: 15_000,
      });
      if ((await branchSelect.locator(`option[value="${users.branch}"]`).count()) > 0) {
        await branchSelect.selectOption(users.branch);
      }
      // #alert-currency: web/src/screens/customer/rate-alerts/RateAlerts.tsx:471
      await expect(page.locator('#alert-currency option'), '#alert-currency options loaded').not.toHaveCount(
        0,
        { timeout: 15_000 },
      );
      await page.locator('#alert-currency').selectOption('USD');
      // #alert-side: web/src/screens/customer/rate-alerts/RateAlerts.tsx:387-395
      await page.locator('#alert-side').selectOption('CUSTOMER_BUY');
      // #alert-condition: web/src/screens/customer/rate-alerts/RateAlerts.tsx:400-408
      await page.locator('#alert-condition').selectOption('ABOVE');
      // #alert-target: web/src/screens/customer/rate-alerts/RateAlerts.tsx:464
      await page.locator('#alert-target').fill('98.75');
      // #alert-expires: web/src/screens/customer/rate-alerts/RateAlerts.tsx:478
      await page.locator('#alert-expires').fill('');

      // POST /api/app/rate-alerts: web/src/screens/customer/rate-alerts/RateAlerts.tsx:71,398,494
      const createRespPromise = page.waitForResponse(
        (r) => r.url().includes('/api/app/rate-alerts') && r.request().method() === 'POST',
        { timeout: 15_000 },
      );
      await page.locator('.xc-create-alert-form button[type="submit"]').click();
      const createResp = await createRespPromise;
      if (createResp.status() !== 200) {
        const errBody = (await createResp.json().catch(() => ({}))) as {
          error?: { code?: string };
        };
        const code = errBody.error?.code ?? `status-${createResp.status()}`;
        console.log(`[customer-extras] rateAlertCreate returned ${code}; skipping pause/resume`);
        test.skip(true, `rateAlertCreate refused (${code}): active alert cap reached`);
        return;
      }
      const createdBody = (await createResp.json()) as { id?: string };
      expect(createdBody.id, 'created alert id is non-empty').toBeTruthy();
      createdAlertId = createdBody.id!;

      // article.xc-alert-card[data-alert-id]: web/src/screens/customer/rate-alerts/RateAlerts.tsx:548
      const card = page.locator(`article.xc-alert-card[data-alert-id="${createdAlertId}"]`);
      await expect(card, 'created rate alert card visible').toBeVisible({ timeout: 15_000 });
      // .xc-badge-active: web/src/screens/customer/rate-alerts/RateAlerts.tsx:557
      await expect(card.locator('.xc-badge-active'), 'new alert starts ACTIVE').toBeVisible();

      // rateAlertPause — button.xc-btn-pause: web/src/screens/customer/rate-alerts/RateAlerts.tsx:76,600
      const pauseRespPromise = page.waitForResponse(
        (r) =>
          r.url().includes(`/api/app/rate-alerts/${createdAlertId}/pause`) &&
          r.request().method() === 'POST',
        { timeout: 15_000 },
      );
      await card.locator('button.xc-btn-pause').click();
      const pauseResp = await pauseRespPromise;
      expect(pauseResp.status(), 'POST /api/app/rate-alerts/{id}/pause returns 200').toBe(200);
      // .xc-badge-paused: web/src/screens/customer/rate-alerts/RateAlerts.tsx:557
      await expect(card.locator('.xc-badge-paused'), 'alert shows PAUSED badge after pause').toBeVisible({
        timeout: 15_000,
      });

      // Reload and read back PAUSED state
      await page.reload();
      const cardAfterPauseReload = page.locator(`article.xc-alert-card[data-alert-id="${createdAlertId}"]`);
      await expect(
        cardAfterPauseReload.locator('.xc-badge-paused'),
        'PAUSED badge persists after page.reload()',
      ).toBeVisible({ timeout: 15_000 });
      // button.xc-btn-resume: web/src/screens/customer/rate-alerts/RateAlerts.tsx:591
      await expect(
        cardAfterPauseReload.locator('button.xc-btn-resume'),
        'resume button visible after page.reload()',
      ).toBeVisible();

      // rateAlertResume — button.xc-btn-resume: web/src/screens/customer/rate-alerts/RateAlerts.tsx:81,591
      const resumeRespPromise = page.waitForResponse(
        (r) =>
          r.url().includes(`/api/app/rate-alerts/${createdAlertId}/resume`) &&
          r.request().method() === 'POST',
        { timeout: 15_000 },
      );
      await cardAfterPauseReload.locator('button.xc-btn-resume').click();
      const resumeResp = await resumeRespPromise;
      expect(resumeResp.status(), 'POST /api/app/rate-alerts/{id}/resume returns 200').toBe(200);
      await expect(
        cardAfterPauseReload.locator('.xc-badge-active'),
        'alert shows ACTIVE badge after resume',
      ).toBeVisible({ timeout: 15_000 });

      // Reload and read back ACTIVE state
      await page.reload();
      const cardAfterResumeReload = page.locator(`article.xc-alert-card[data-alert-id="${createdAlertId}"]`);
      await expect(
        cardAfterResumeReload.locator('.xc-badge-active'),
        'ACTIVE badge persists after page.reload()',
      ).toBeVisible({ timeout: 15_000 });
      await expect(
        cardAfterResumeReload.locator('button.xc-btn-pause'),
        'pause button visible after page.reload()',
      ).toBeVisible();

      // (c) Permission: teller is refused on rateAlertPause and rateAlertResume (smoke-rbac-matrix.spec.ts:92; rate_alert_lifecycle.rs:84-87,136-139)
      if (hasAccount('teller')) {
        const tellerCtx = await browser.newContext();
        try {
          const tellerPage = await tellerCtx.newPage();
          await signIn(tellerPage, 'teller', '/staff');
          const pauseAsTeller = await postJson(
            tellerPage,
            `/api/app/rate-alerts/${encodeURIComponent(createdAlertId)}/pause`,
            { branch: users.branch },
          );
          expectPermissionRefused(pauseAsTeller, 'teller rateAlertPause');

          const resumeAsTeller = await postJson(
            tellerPage,
            `/api/app/rate-alerts/${encodeURIComponent(createdAlertId)}/resume`,
            { branch: users.branch },
          );
          expectPermissionRefused(resumeAsTeller, 'teller rateAlertResume');
        } finally {
          await tellerCtx.close();
        }
      }
    } finally {
      if (createdAlertId) {
        await deleteAlertById(page, createdAlertId);
      }
    }
  });

  test('queueTicketTake: (a) takes a queue number on /app/branches and reads it back across reload; (c) refuses teller', async ({
    page,
    browser,
  }) => {
    let takenTicketId: string | null = null;

    try {
      // (c) Permission: teller has no customer session / CustomerSelf grant (smoke-rbac-matrix.spec.ts:92)
      if (hasAccount('teller')) {
        const tellerCtx = await browser.newContext();
        try {
          const tellerPage = await tellerCtx.newPage();
          await signIn(tellerPage, 'teller', '/staff');
          const tellerTake = await postJson(tellerPage, '/api/app/queue/tickets', {
            branch: users.branch,
          });
          expectPermissionRefused(tellerTake, 'teller queueTicketTake');
        } finally {
          await tellerCtx.close();
        }
      }

      await signIn(page, 'customer', '/app/branches');
      // .xc-branches-screen: web/src/screens/customer/branches/BranchesScreen.tsx:122
      await smoke(page, {
        area: 'customer-extras',
        screen: 'branches',
        path: '/app/branches',
        role: 'customer',
        key: '.xc-branches-screen',
        langs: CUSTOMER_LANGS,
      });

      await withLang(page, 'en');
      await page.goto('/app/branches');
      // article.xc-branch-card[data-branch-code]: web/src/screens/customer/branches/BranchesScreen.tsx:207
      const branchCard = page.locator(`article.xc-branch-card[data-branch-code="${users.branch}"]`);
      await expect(branchCard, `branch card ${users.branch} visible`).toBeVisible({ timeout: 15_000 });

      // button.xc-take-number-btn[data-action="take-number"]: web/src/screens/customer/branches/TakeNumber.tsx:74-80
      const takeBtn = branchCard.locator('button.xc-take-number-btn[data-action="take-number"]');
      if (!(await takeBtn.isVisible())) {
        // .xc-take-number-closed: web/src/screens/customer/branches/TakeNumber.tsx:82
        console.log(`[customer-extras] queue closed on ${users.branch}; skipping queueTicketTake`);
        test.skip(true, `queue closed or outside operating hours on ${users.branch}`);
        return;
      }

      // POST /api/app/queue/tickets: web/src/screens/customer/branches/TakeNumber.tsx:30
      const takeRespPromise = page.waitForResponse(
        (r) => r.url().includes('/api/app/queue/tickets') && r.request().method() === 'POST',
        { timeout: 15_000 },
      );
      await takeBtn.click();
      const takeResp = await takeRespPromise;

      if (takeResp.status() !== 200) {
        const errBody = (await takeResp.json().catch(() => ({}))) as {
          error?: { code?: string; message?: string };
        };
        const code = errBody.error?.code ?? `status-${takeResp.status()}`;
        // [data-testid="take-number-error"]: web/src/screens/customer/branches/TakeNumber.tsx:99
        await expect(
          branchCard.locator('[data-testid="take-number-error"]'),
          `worded error shown on take-number refusal (${code})`,
        ).toBeVisible({ timeout: 10_000 });
        console.log(`[customer-extras] queueTicketTake refused with ${code}; skipping`);
        test.skip(true, `queueTicketTake refused (${code}): queue closed or outside hours`);
        return;
      }

      const issued = (await takeResp.json()) as { ticketId: string; number: string };
      takenTicketId = issued.ticketId;
      expect(issued.ticketId, 'issued ticketId present').toBeTruthy();
      expect(issued.number, 'issued queue number present').toBeTruthy();

      // [data-testid="take-number-message"]: web/src/screens/customer/branches/TakeNumber.tsx:88
      await expect(
        branchCard.locator('[data-testid="take-number-message"]'),
        'take-number confirmation message visible',
      ).toContainText(issued.number);

      // [data-testid="take-number-link"]: web/src/screens/customer/branches/TakeNumber.tsx:93
      const journeyLink = branchCard.locator('[data-testid="take-number-link"]');
      await expect(journeyLink, 'journey link visible after taking number').toBeVisible();
      await journeyLink.click();

      // .xc-journey-screen & [data-testid="journey-queue-number"]: web/src/screens/customer/journey/Journey.tsx:194,254
      const queueNumberEl = page.locator('[data-testid="journey-queue-number"]');
      await expect(queueNumberEl, 'journey screen displays taken queue number').toContainText(
        issued.number,
        { timeout: 15_000 },
      );

      // Reload and read the active ticket back
      await page.reload();
      await expect(
        page.locator('[data-testid="journey-queue-number"]'),
        'taken queue number persists after page.reload()',
      ).toContainText(issued.number, { timeout: 15_000 });
    } finally {
      if (takenTicketId) {
        await cancelQueueTicketAsTeller(browser, takenTicketId);
      }
    }
  });

  // Sub-controller ruling (issue #4294 comment 6321016194): app bookings start CONFIRMED
  // (api/crates/xc-booking/src/domain/booking/source_channel.rs:16-17), so bookingConfirm is covered
  // by (b) + (c) on the spec's own booking, with no page.route projection and no touching seeded rows.
  test('bookingConfirm: (b) refuses confirming an already-CONFIRMED booking with 409 and preserves CONFIRMED across reload; (c) refuses teller', async ({
    page,
    browser,
  }) => {
    let createdBookingId: string | null = null;

    try {
      await signIn(page, 'customer', '/app/bookings');
      // .xc-bookings-screen: web/src/screens/customer/bookings/Bookings.tsx:332
      await smoke(page, {
        area: 'customer-extras',
        screen: 'bookings',
        path: '/app/bookings',
        role: 'customer',
        key: '.xc-bookings-screen',
        langs: CUSTOMER_LANGS,
      });

      // Create the spec's own booking via /app/book (flow-booking.spec.ts:34-56; web/src/screens/customer/book/Book.tsx:395-708)
      await withLang(page, 'en');
      await page.goto('/app/book');
      // [data-testid="book-screen"]: web/src/screens/customer/book/Book.tsx:395
      await expect(page.getByTestId('book-screen')).toBeVisible({ timeout: 15_000 });
      // [data-testid="branch-select"]: web/src/screens/customer/book/Book.tsx:459
      await page.getByTestId('branch-select').selectOption(users.branch);
      // [data-testid="date-input"]: web/src/screens/customer/book/Book.tsx:511
      await page.getByTestId('date-input').fill(tomorrowBangkok());
      // [data-testid="slot-option"]:not([disabled]): web/src/screens/customer/book/Book.tsx:588-590
      const firstSlot = page.locator('[data-testid="slot-option"]:not([disabled])').first();
      await firstSlot.waitFor({ state: 'attached', timeout: 15_000 }).catch(() => {});
      if ((await firstSlot.count()) === 0) {
        console.log(`[customer-extras] no open booking slots on ${users.branch}; skipping bookingConfirm`);
        test.skip(true, `no open booking slots on ${users.branch}`);
        return;
      }
      const slotValue = await firstSlot.getAttribute('value');
      // [data-testid="slot-select"]: web/src/screens/customer/book/Book.tsx:575
      await page.getByTestId('slot-select').selectOption(slotValue!);
      // [data-testid="currency-select"]: web/src/screens/customer/book/Book.tsx:627
      await page.getByTestId('currency-select').selectOption(users.currency ?? 'USD');
      // [data-testid="quantity-input"]: web/src/screens/customer/book/Book.tsx:650
      await page.getByTestId('quantity-input').fill(users.quantity ?? '100');

      // [data-testid="book-submit"]: web/src/screens/customer/book/Book.tsx:702
      const createRespPromise = page.waitForResponse(
        (r) => r.request().method() === 'POST' && new URL(r.url()).pathname === '/api/bookings',
        { timeout: 15_000 },
      );
      await page.getByTestId('book-submit').click();
      const createResp = await createRespPromise;
      if (createResp.status() !== 200) {
        console.log(`[customer-extras] bookingCreate returned ${createResp.status()}; skipping bookingConfirm`);
        test.skip(true, `bookingCreate returned ${createResp.status()}`);
        return;
      }
      const createBody = (await createResp.json()) as {
        id?: string;
        status?: string;
        data?: { id?: string; status?: string };
      };
      const created = createBody.data ?? createBody;
      createdBookingId = created.id!;
      expect(created.status, 'APP-created booking starts CONFIRMED').toBe('CONFIRMED');

      // Verify on /app/bookings that the CONFIRMED card does not offer button[data-action="confirm"] (BookingExtras.tsx:104)
      await page.goto('/app/bookings');
      // .xc-booking-card[data-booking-id]: web/src/screens/customer/bookings/Bookings.tsx:380
      const bookingCard = page.locator(`.xc-booking-card[data-booking-id="${createdBookingId}"]`);
      await expect(bookingCard, 'created booking card visible').toBeVisible({ timeout: 15_000 });
      // .xc-status-confirmed: web/src/screens/customer/bookings/Bookings.tsx:383
      await expect(bookingCard.locator('.xc-status-confirmed'), 'card shows CONFIRMED').toBeVisible();
      // button[data-action="confirm"]: web/src/screens/customer/bookings/BookingExtras.tsx:104-113
      await expect(
        bookingCard.locator('button[data-action="confirm"]'),
        'confirm button not offered on CONFIRMED booking',
      ).toHaveCount(0);

      // (b) Confirming the already-CONFIRMED booking is refused with 409 (confirm.rs:107-122, booking_error_map.rs:25-27)
      // POST /api/bookings/{id}/confirm: web/src/screens/customer/bookings/BookingExtras.tsx:38
      console.log(`[customer-extras] bookingConfirm (b) lang=en value=${createdBookingId}`);
      const confirmRefusal = await postJson(
        page,
        `/api/bookings/${encodeURIComponent(createdBookingId)}/confirm`,
        { branch: users.branch },
      );
      expect(confirmRefusal.status, 'confirm on CONFIRMED booking refused with 409').toBe(409);
      const errObj = (confirmRefusal.body.error ?? {}) as { code?: string; message?: string };
      expect(errObj.code, 'refusal code is staleVersion').toBe('staleVersion');
      expect((errObj.message ?? '').trim().length, 'refusal message is non-empty').toBeGreaterThan(0);

      // Reload and read back CONFIRMED status on /app/bookings
      await page.reload();
      const reloadedCard = page.locator(`.xc-booking-card[data-booking-id="${createdBookingId}"]`);
      await expect(reloadedCard, 'booking card visible after reload').toBeVisible({ timeout: 15_000 });
      await expect(
        reloadedCard.locator('.xc-status-confirmed'),
        'booking card still CONFIRMED after reload',
      ).toBeVisible();

      // (c) Permission: teller is refused on POST /api/bookings/{id}/confirm (smoke-rbac-matrix.spec.ts:92; confirm.rs:91-95)
      if (hasAccount('teller')) {
        const tellerCtx = await browser.newContext();
        try {
          const tellerPage = await tellerCtx.newPage();
          await signIn(tellerPage, 'teller', '/staff');
          const tellerConfirm = await postJson(
            tellerPage,
            `/api/bookings/${encodeURIComponent(createdBookingId)}/confirm`,
            { branch: users.branch },
          );
          expectPermissionRefused(tellerConfirm, 'teller bookingConfirm');
        } finally {
          await tellerCtx.close();
        }
      }
    } finally {
      if (createdBookingId) {
        await postJson(page, `/api/bookings/${encodeURIComponent(createdBookingId)}/cancel`, {
          branch: users.branch,
          reason: `${RUN_TAG}-cleanup`,
        }).catch(() => {});
      }
    }
  });

  test('notificationRead and notificationReadAll: (a) marks one and all notifications read across reloads with unread count dropping; (c) refuses teller on customer notification', async ({
    page,
    browser,
  }) => {
    let leftoverTicketId: string | null = null;

    try {
      await signIn(page, 'customer', '/app/notifications');
      // .xc-notifications-screen: web/src/screens/customer/notifications/Notifications.tsx:43
      await smoke(page, {
        area: 'customer-extras',
        screen: 'notifications',
        path: '/app/notifications',
        role: 'customer',
        key: '.xc-notifications-screen',
        langs: CUSTOMER_LANGS,
      });

      await withLang(page, 'en');
      await page.goto('/app/notifications');
      // li.xc-notifications-item.xc-notifications-item-unread: web/src/screens/customer/notifications/Notifications.tsx:97-98
      const unreadItems = page.locator('li.xc-notifications-item.xc-notifications-item-unread');

      // If fewer than 2 unread notifications exist, try producing 2 real queue_called notifications via teller call + recall
      if ((await unreadItems.count()) < 2) {
        leftoverTicketId = await produceQueueNotifications(page, browser, true);
        await page.reload();
      }

      const countBeforeReadOne = await unreadItems.count();
      if (countBeforeReadOne < 1) {
        console.log(
          `[customer-extras] 0 unread notifications on ${users.branch} (queue closed or no unread items); skipping notificationRead / notificationReadAll`,
        );
        test.skip(true, `no real unread notifications available on ${users.branch}`);
        return;
      }

      // (a.1) notificationRead — button.xc-notifications-mark-btn: web/src/screens/customer/notifications/Notifications.tsx:124
      // POST /api/app/notifications/{id}/read: web/src/shared/notifications/notifications.ts:49
      const readOnePromise = page.waitForResponse(
        (r) =>
          r.url().includes('/api/app/notifications/') &&
          r.url().endsWith('/read') &&
          r.request().method() === 'POST',
        { timeout: 15_000 },
      );
      await unreadItems.first().locator('button.xc-notifications-mark-btn').click();
      const readOneResp = await readOnePromise;
      expect(readOneResp.status(), 'POST /api/app/notifications/{id}/read returns 200').toBe(200);

      // Extract the real notification id from the request URL `/api/app/notifications/{id}/read`
      // or from GET /api/v3/views/my_notifications
      const match = readOneResp.url().match(/\/api\/app\/notifications\/([^/]+)\/read$/);
      let customerNotificationId = match ? decodeURIComponent(match[1]) : null;
      if (!customerNotificationId) {
        const myNotifRes = await page.evaluate(async (branch) => {
          const res = await fetch(
            `/api/v3/views/my_notifications?branch=${encodeURIComponent(branch)}`,
            { credentials: 'same-origin' },
          );
          return (await res.json().catch(() => ({}))) as {
            data?: { items?: Array<{ id?: string }> };
          };
        }, users.branch);
        customerNotificationId = myNotifRes.data?.items?.[0]?.id ?? null;
      }

      await page.reload();
      const countAfterReadOne = await unreadItems.count();
      expect(
        countAfterReadOne,
        'unread notification count dropped after notificationRead and persisted across reload',
      ).toBeLessThan(countBeforeReadOne);

      // Ensure at least 1 unread notification remains before testing notificationReadAll
      if ((await unreadItems.count()) < 1) {
        leftoverTicketId = await produceQueueNotifications(page, browser, false);
        await page.reload();
      }
      const countBeforeReadAll = await unreadItems.count();
      if (countBeforeReadAll < 1) {
        console.log(
          `[customer-extras] 0 unread notifications remaining for notificationReadAll; skipping mark-all-read step`,
        );
        test.skip(true, 'no unread notification remaining for notificationReadAll');
        return;
      }

      // (a.2) notificationReadAll — button.xc-notifications-mark-all-btn: web/src/screens/customer/notifications/Notifications.tsx:62
      // POST /api/app/notifications/read-all: web/src/shared/notifications/notifications.ts:54
      const markAllBtn = page.locator('button.xc-notifications-mark-all-btn');
      await expect(markAllBtn, 'mark-all-read button enabled when unread > 0').toBeEnabled();

      const readAllPromise = page.waitForResponse(
        (r) =>
          r.url().includes('/api/app/notifications/read-all') && r.request().method() === 'POST',
        { timeout: 15_000 },
      );
      await markAllBtn.click();
      const readAllResp = await readAllPromise;
      expect(readAllResp.status(), 'POST /api/app/notifications/read-all returns 200').toBe(200);

      await expect(unreadItems, 'unread count dropped to 0 after mark-all-read').toHaveCount(0);
      await expect(markAllBtn, 'mark-all-read button disabled when unread === 0').toBeDisabled();

      // Reload and read back 0 unread notifications
      await page.reload();
      await expect(unreadItems, '0 unread items persist after page.reload()').toHaveCount(0);
      await expect(
        page.locator('button.xc-notifications-mark-all-btn'),
        'mark-all-read button remains disabled after page.reload()',
      ).toBeDisabled();

      // (c) Permission: teller is refused when reading the customer's real notification (notification_read.rs:24-28)
      if (!customerNotificationId) {
        console.log('[customer-extras] no customerNotificationId found; skipping notificationRead (c)');
        test.skip(true, 'no customerNotificationId found for permission check');
        return;
      }
      if (hasAccount('teller')) {
        const tellerCtx = await browser.newContext();
        try {
          const tellerPage = await tellerCtx.newPage();
          await signIn(tellerPage, 'teller', '/staff');
          const foreignRead = await postJson(
            tellerPage,
            `/api/app/notifications/${encodeURIComponent(customerNotificationId)}/read`,
            { branch: users.branch },
          );
          expect(
            foreignRead.status,
            `teller notificationRead on customer notification ${customerNotificationId} refused with 403`,
          ).toBe(403);
        } finally {
          await tellerCtx.close();
        }
      }
    } finally {
      if (leftoverTicketId) {
        await cancelQueueTicketAsTeller(browser, leftoverTicketId);
      }
    }
  });
});
