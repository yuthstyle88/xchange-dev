// BK-9 (#1410): Admin Booking Slots screen (design `docs/design/blueprint/booking-slots.md` §4.4, §7, §8, §9 BK-9).
import { createEffect, createSignal, For, Show, type JSX } from 'solid-js';
import { ApiClient, ApiError } from '../../../shared/api/client';
import { keyedWrite } from '../../../shared/api/keyed-write';
import { locale, screenT } from '../../../shared/i18n';
import { fieldError } from '../../../shared/i18n/field-error';
import { isServerFailure, wordedError } from '../../../shared/i18n/worded-error';
import { sharedSession } from '../../../shared/session/session';
import type { AdminMenuProps } from '../menus';
import type {
  AdminBookingSlotsData,
  AdminClosureDto,
  AdminSlotDto,
} from '../../../shared/api/generated/admin_booking_slots';
import type {
  BookingPolicySetInput,
  ClosureAddInput,
  SlotCapacitySetInput,
  SlotRuleSetInput,
} from '../../../shared/api/generated/bookings';
import * as table from './i18n';
import './booking-slots.css';

function formatSlotTime(iso: string): string {
  if (!iso) return iso;
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  return d.toLocaleString(locale(), {
    timeZone: 'Asia/Bangkok',
    hour: '2-digit',
    minute: '2-digit',
    hour12: false,
    hourCycle: 'h23',
  });
}

export function BookingSlots(props: AdminMenuProps): JSX.Element {
  const t = screenT('admin.booking-slots', table);
  const api = new ApiClient();
  const writes = new Map<string, { send(body: unknown): Promise<unknown>; reset(): void }>();

  function writeFor<B, R>(actionId: string, method: string, path: string): { send(body: B): Promise<R>; reset(): void } {
    let w = writes.get(actionId);
    if (!w) {
      w = keyedWrite<unknown, unknown>(api, method, path);
      writes.set(actionId, w);
    }
    return w as { send(body: B): Promise<R>; reset(): void };
  }

  const [loading, setLoading] = createSignal(false);
  const [errorMessage, setErrorMessage] = createSignal<string | null>(null);
  const [fieldErrors, setFieldErrors] = createSignal<Record<string, string>>({});
  // #2427: one write at a time, so a second click cannot resend the same idempotency key while the first is in flight.
  const [writing, setWriting] = createSignal(false);

  // Slot Rule
  const [slotMinutes, setSlotMinutes] = createSignal('');
  const [capacity, setCapacity] = createSignal('');
  const [horizonDays, setHorizonDays] = createSignal('');

  // Closures
  const [closures, setClosures] = createSignal<AdminClosureDto[]>([]);
  const [newClosedOn, setNewClosedOn] = createSignal('');
  const [newReason, setNewReason] = createSignal('');

  // Slots
  const [slots, setSlots] = createSignal<AdminSlotDto[]>([]);
  const [slotCapacities, setSlotCapacities] = createSignal<Record<string, string>>({});

  // Policy
  const [cutoff, setCutoff] = createSignal('');
  const [grace, setGrace] = createSignal('');
  const [expiry, setExpiry] = createSignal('');

  let latestReadId = 0;

  function clearBranchState() {
    setSlotMinutes('');
    setCapacity('');
    setHorizonDays('');
    setClosures([]);
    setSlots([]);
    setSlotCapacities({});
    setFieldErrors({});
  }

  async function loadView(branchId: string) {
    const readId = ++latestReadId;
    setLoading(true);
    setErrorMessage(null);
    setFieldErrors({});
    try {
      const res = await api.view<AdminBookingSlotsData>('admin_booking_slots', { branch: branchId });
      if (readId !== latestReadId) return;
      const data = res.data;
      if (data) {
        if (data.rule) {
          setSlotMinutes(String(data.rule.slotMinutes));
          setCapacity(String(data.rule.capacity));
          setHorizonDays(String(data.rule.horizonDays));
        } else {
          setSlotMinutes('');
          setCapacity('');
          setHorizonDays('');
        }
        setClosures(data.closures ?? []);
        setSlots(data.slots ?? []);
        if (data.policy) {
          setCutoff(String(data.policy.cancelCutoffMinutes));
          setGrace(String(data.policy.noShowGraceMinutes));
          setExpiry(String(data.policy.confirmExpiryMinutes));
        }
        const caps: Record<string, string> = {};
        for (const s of data.slots ?? []) {
          caps[s.id] = String(s.capacity);
        }
        setSlotCapacities(caps);
      }
    } catch (err: unknown) {
      if (readId !== latestReadId) return;
      clearBranchState();
      handleApiError(err);
    } finally {
      if (readId === latestReadId) {
        setLoading(false);
      }
    }
  }

  createEffect(() => {
    const branch = props.branch();
    if (branch) {
      clearBranchState();
      void loadView(branch);
    }
  });

  const FIELD_MAP: Record<string, string> = {
    slotMinutes: 'slotMinutes',
    slot_minutes: 'slotMinutes',
    capacity: 'capacity',
    horizonDays: 'horizonDays',
    horizon_days: 'horizonDays',
    closedOn: 'closedOn',
    closed_on: 'closedOn',
    reason: 'reason',
    cancelCutoffMinutes: 'cancelCutoffMinutes',
    cancel_cutoff_minutes: 'cancelCutoffMinutes',
    noShowGraceMinutes: 'noShowGraceMinutes',
    no_show_grace_minutes: 'noShowGraceMinutes',
    confirmExpiryMinutes: 'confirmExpiryMinutes',
    confirm_expiry_minutes: 'confirmExpiryMinutes',
  };

  function clearErrors() {
    setErrorMessage(null);
    setFieldErrors({});
  }

  function boundedFieldLabels(): Record<string, string> {
    return {
      reason: t('reason'),
      cancelCutoffMinutes: t('cancelCutoffMinutes'),
      noShowGraceMinutes: t('noShowGraceMinutes'),
      confirmExpiryMinutes: t('confirmExpiryMinutes'),
    };
  }

  function localBoundedFieldError(field: string): string {
    return fieldError(
      new ApiError({ status: 400, code: 'validationFailed', field, message: '' }),
      boundedFieldLabels(),
    ).message;
  }

  function handleApiError(err: unknown) {
    let msg: string;
    if (err instanceof ApiError) {
      const mappedField = err.field ? FIELD_MAP[err.field] : undefined;
      const labels = boundedFieldLabels();
      if (mappedField && Object.prototype.hasOwnProperty.call(labels, mappedField) && !isServerFailure(err)) {
        const fe = fieldError(
          new ApiError({ status: err.status, code: err.code, field: mappedField, message: err.message }),
          labels,
        );
        setFieldErrors({ [mappedField]: fe.message });
        return;
      }
      const code = err.code;
      const errorKey = `error.${code}`;
      const errKey = `err.${code}`;
      if (errorKey in table.th) {
        msg = t(errorKey as any);
      } else if (errKey in table.th) {
        msg = t(errKey as any);
      } else if (!isServerFailure(err)) {
        msg = t('error.validationFailed');
      } else {
        msg = wordedError(err);
      }
      if (mappedField) {
        setFieldErrors({ [mappedField]: msg });
        return;
      }
    } else {
      msg = wordedError(err);
    }
    setErrorMessage(msg);
  }

  async function handleSaveRule() {
    if (writing()) return;
    clearErrors();
    const sMin = Number(slotMinutes());
    const cap = Number(capacity());
    const hDays = Number(horizonDays());

    // Boundary checks: slot_minutes 5-240, capacity > 0, horizon_days 1-90
    if (
      !Number.isInteger(sMin) ||
      sMin < 5 ||
      sMin > 240 ||
      !Number.isInteger(cap) ||
      cap <= 0 ||
      !Number.isInteger(hDays) ||
      hDays < 1 ||
      hDays > 90
    ) {
      setErrorMessage(t('ruleValidationFailed'));
      return;
    }

    const branch = props.branch();
    const payload = {
      slotMinutes: sMin,
      capacity: cap,
      horizonDays: hDays,
      branch,
    } satisfies Omit<SlotRuleSetInput, 'id'> & { branch: string };
    const actionId = `slotRuleSet:${branch}`;
    const path = `/api/branches/${encodeURIComponent(branch)}/booking-slot-rule`;

    setWriting(true);
    try {
      await writeFor(actionId, 'PUT', path).send(payload);
      await loadView(props.branch());
    } catch (err: unknown) {
      if (props.branch() === branch) {
        handleApiError(err);
      }
    } finally {
      setWriting(false);
    }
  }

  async function handleAddClosure() {
    if (writing()) return;
    clearErrors();
    const closedOn = newClosedOn().trim();
    const reason = newReason().trim();

    if (!closedOn || !reason) {
      setErrorMessage(t('closureValidationFailed'));
      return;
    }
    if (newReason().length > 200) {
      setFieldErrors({ reason: localBoundedFieldError('reason') });
      return;
    }

    const branch = props.branch();
    const payload = {
      closedOn,
      reason,
      branch,
    } satisfies Omit<ClosureAddInput, 'id'> & { branch: string };
    const actionId = `closureAdd:${branch}:${closedOn}`;
    const path = `/api/branches/${encodeURIComponent(branch)}/closures`;

    setWriting(true);
    try {
      await writeFor(actionId, 'POST', path).send(payload);
      if (props.branch() === branch) {
        setNewClosedOn('');
        setNewReason('');
      }
      await loadView(props.branch());
    } catch (err: unknown) {
      if (props.branch() === branch) {
        handleApiError(err);
      }
    } finally {
      setWriting(false);
    }
  }

  async function handleRemoveClosure(closedOn: string) {
    if (writing()) return;
    clearErrors();
    const branch = props.branch();
    const payload = { branch };
    const actionId = `closureRemove:${branch}:${closedOn}`;
    const path = `/api/branches/${encodeURIComponent(branch)}/closures/${encodeURIComponent(closedOn)}/remove`;

    setWriting(true);
    try {
      await writeFor(actionId, 'POST', path).send(payload);
      await loadView(props.branch());
    } catch (err: unknown) {
      if (props.branch() === branch) {
        handleApiError(err);
      }
    } finally {
      setWriting(false);
    }
  }

  async function handleUpdateSlotCapacity(slotId: string) {
    if (writing()) return;
    clearErrors();
    const capVal = slotCapacities()[slotId];
    const capNum = Number(capVal);

    if (!Number.isInteger(capNum) || capNum <= 0) {
      setErrorMessage(t('slotCapacityValidationFailed'));
      return;
    }

    const branch = props.branch();
    const payload = {
      capacity: capNum,
      branch,
    } satisfies Omit<SlotCapacitySetInput, 'id'> & { branch: string };
    const actionId = `slotCapacitySet:${slotId}`;
    const path = `/api/booking-slots/${encodeURIComponent(slotId)}/capacity`;

    setWriting(true);
    try {
      await writeFor(actionId, 'PUT', path).send(payload);
      await loadView(props.branch());
    } catch (err: unknown) {
      if (props.branch() === branch) {
        handleApiError(err);
      }
    } finally {
      setWriting(false);
    }
  }

  async function handleCloseSlot(slotId: string) {
    if (writing()) return;
    clearErrors();
    const branch = props.branch();
    const payload = { branch };
    const actionId = `slotClose:${slotId}`;
    const path = `/api/booking-slots/${encodeURIComponent(slotId)}/close`;

    setWriting(true);
    try {
      await writeFor(actionId, 'POST', path).send(payload);
      await loadView(props.branch());
    } catch (err: unknown) {
      if (props.branch() === branch) {
        handleApiError(err);
      }
    } finally {
      setWriting(false);
    }
  }

  async function handleOpenSlot(slotId: string) {
    if (writing()) return;
    clearErrors();
    const branch = props.branch();
    const payload = { branch };
    const actionId = `slotOpen:${slotId}`;
    const path = `/api/booking-slots/${encodeURIComponent(slotId)}/open`;

    setWriting(true);
    try {
      await writeFor(actionId, 'POST', path).send(payload);
      await loadView(props.branch());
    } catch (err: unknown) {
      if (props.branch() === branch) {
        handleApiError(err);
      }
    } finally {
      setWriting(false);
    }
  }

  async function handleSavePolicy() {
    if (writing()) return;
    clearErrors();
    const cutRaw = cutoff().trim();
    const grcRaw = grace().trim();
    const expRaw = expiry().trim();
    if (!/^\d+$/.test(cutRaw) || !/^\d+$/.test(grcRaw) || !/^\d+$/.test(expRaw) || BigInt(expRaw) <= 0n) {
      setErrorMessage(t('policyValidationFailed'));
      return;
    }
    const overMaxErrs: Record<string, string> = {};
    if (BigInt(cutRaw) > 129600n) {
      overMaxErrs.cancelCutoffMinutes = localBoundedFieldError('cancelCutoffMinutes');
    }
    if (BigInt(grcRaw) > 129600n) {
      overMaxErrs.noShowGraceMinutes = localBoundedFieldError('noShowGraceMinutes');
    }
    if (BigInt(expRaw) > 129600n) {
      overMaxErrs.confirmExpiryMinutes = localBoundedFieldError('confirmExpiryMinutes');
    }
    if (Object.keys(overMaxErrs).length > 0) {
      setFieldErrors(overMaxErrs);
      return;
    }
    const cut = Number(cutRaw);
    const grc = Number(grcRaw);
    const exp = Number(expRaw);

    const payload = {
      cancelCutoffMinutes: cut,
      noShowGraceMinutes: grc,
      confirmExpiryMinutes: exp,
      branch: props.branch(),
    } satisfies BookingPolicySetInput & { branch: string };
    const actionId = 'bookingPolicySet';

    setWriting(true);
    try {
      await writeFor(actionId, 'PUT', '/api/booking-policy').send(payload);
      await loadView(props.branch());
    } catch (err: unknown) {
      handleApiError(err);
    } finally {
      setWriting(false);
    }
  }

  // shortcut: me cannot tell which role row grants BranchesManage; errs toward showing, upgrade when me exposes orgPermissions
  const mayEditBookingPolicy = () =>
    Boolean(
      sharedSession().me()?.permissions?.includes('BranchesManage') &&
        sharedSession().me()?.roles?.some((r) => r.branchId === null),
    );

  return (
    <div class="booking-slots-screen">
      <Show when={loading()}>
        <div class="loading">{t('loading')}</div>
      </Show>

      <Show when={errorMessage()}>
        <div role="alert" class="error field-error">
          {errorMessage()}
        </div>
      </Show>

      {/* Section 1: Slot Rule */}
      <section class="booking-slots-section">
        <h3>{t('ruleSection')}</h3>
        <div class="form-group">
          <label for="slotMinutes">{t('slotMinutes')}</label>
          <input
            id="slotMinutes"
            name="slotMinutes"
            data-field="slotMinutes"
            type="number"
            aria-invalid={fieldErrors().slotMinutes ? 'true' : undefined}
            value={slotMinutes()}
            onInput={(e) => setSlotMinutes(e.currentTarget.value)}
          />
          <Show when={fieldErrors().slotMinutes}>
            {(msg) => (
              <div role="alert" class="error field-error" data-error-for="slotMinutes">
                {msg()}
              </div>
            )}
          </Show>
        </div>
        <div class="form-group">
          <label for="capacity">{t('capacity')}</label>
          <input
            id="capacity"
            name="capacity"
            data-field="capacity"
            type="number"
            aria-invalid={fieldErrors().capacity ? 'true' : undefined}
            value={capacity()}
            onInput={(e) => setCapacity(e.currentTarget.value)}
          />
          <Show when={fieldErrors().capacity}>
            {(msg) => (
              <div role="alert" class="error field-error" data-error-for="capacity">
                {msg()}
              </div>
            )}
          </Show>
        </div>
        <div class="form-group">
          <label for="horizonDays">{t('horizonDays')}</label>
          <input
            id="horizonDays"
            name="horizonDays"
            data-field="horizonDays"
            type="number"
            aria-invalid={fieldErrors().horizonDays ? 'true' : undefined}
            value={horizonDays()}
            onInput={(e) => setHorizonDays(e.currentTarget.value)}
          />
          <Show when={fieldErrors().horizonDays}>
            {(msg) => (
              <div role="alert" class="error field-error" data-error-for="horizonDays">
                {msg()}
              </div>
            )}
          </Show>
        </div>
        <button type="button" disabled={writing()} onClick={handleSaveRule}>
          {t('saveRule')}
        </button>
      </section>

      {/* Section 2: Branch Closures */}
      <section class="booking-slots-section">
        <h3>{t('closuresSection')}</h3>
        <div class="closures-form">
          <div class="form-group">
            <label for="closedOn">{t('closedOn')}</label>
            <input
              id="closedOn"
              name="closedOn"
              data-field="closedOn"
              type="date"
              aria-invalid={fieldErrors().closedOn ? 'true' : undefined}
              value={newClosedOn()}
              onInput={(e) => setNewClosedOn(e.currentTarget.value)}
            />
            <Show when={fieldErrors().closedOn}>
              {(msg) => (
                <div role="alert" class="error field-error" data-error-for="closedOn">
                  {msg()}
                </div>
              )}
            </Show>
          </div>
          <div class="form-group">
            <label for="reason">{t('reason')}</label>
            <input
              id="reason"
              name="reason"
              data-field="reason"
              type="text"
              maxLength={200}
              aria-invalid={fieldErrors().reason ? 'true' : undefined}
              value={newReason()}
              onInput={(e) => setNewReason(e.currentTarget.value)}
            />
            <div>{newReason().length} / 200</div>
            <Show when={fieldErrors().reason}>
              {(msg) => (
                <div role="alert" class="error field-error" data-error-for="reason" data-field-error="reason">
                  {msg()}
                </div>
              )}
            </Show>
          </div>
          <button type="button" disabled={writing()} onClick={handleAddClosure}>
            {t('addClosure')}
          </button>
        </div>

        <div class="closures-list">
          <For each={closures()} fallback={<div>{t('noClosures')}</div>}>
            {(closure) => (
              <div class="closure-item">
                <span class="closure-date">{closure.closedOn}</span>
                <span class="closure-reason">{closure.reason}</span>
                <button type="button" disabled={writing()} onClick={() => void handleRemoveClosure(closure.closedOn)}>
                  {t('removeClosure')}
                </button>
              </div>
            )}
          </For>
        </div>
      </section>

      {/* Section 3: Booking Slots */}
      <section class="booking-slots-section">
        <h3>{t('slotsSection')}</h3>
        <div class="slots-list">
          <For each={slots()} fallback={<div>{t('noSlots')}</div>}>
            {(slot) => (
              <div class="slot-item" data-slot-id={slot.id}>
                <span class="slot-date">{slot.serviceDate}</span>
                <span class="slot-time">
                  {formatSlotTime(slot.startAt)} - {formatSlotTime(slot.endAt)}
                </span>
                <span class="slot-status">{slot.status}</span>
                <span class="slot-reserved">
                  {t('reserved')}: {slot.reserved}
                </span>
                <div class="slot-capacity-control">
                  <input
                    name="slotCapacity"
                    data-slot-id={slot.id}
                    type="number"
                    value={slotCapacities()[slot.id] ?? slot.capacity}
                    onInput={(e) => {
                      setSlotCapacities((prev) => ({
                        ...prev,
                        [slot.id]: e.currentTarget.value,
                      }));
                    }}
                  />
                  <button type="button" disabled={writing()} onClick={() => void handleUpdateSlotCapacity(slot.id)}>
                    {t('updateCapacity')}
                  </button>
                </div>
                <button type="button" disabled={writing()} onClick={() => void handleCloseSlot(slot.id)}>
                  {t('close')}
                </button>
                <button type="button" disabled={writing()} onClick={() => void handleOpenSlot(slot.id)}>
                  {t('open')}
                </button>
              </div>
            )}
          </For>
        </div>
      </section>

      {/* Section 4: Booking Policy */}
      <Show when={mayEditBookingPolicy()}>
        <section class="booking-slots-section">
          <h3>{t('policySection')}</h3>
          <div class="form-group">
            <label for="cancelCutoffMinutes">{t('cancelCutoffMinutes')}</label>
            <input
              id="cancelCutoffMinutes"
              name="cancelCutoffMinutes"
              data-field="cancelCutoffMinutes"
              type="number"
              min="0"
              max="129600"
              aria-invalid={fieldErrors().cancelCutoffMinutes ? 'true' : undefined}
              value={cutoff()}
              onInput={(e) => setCutoff(e.currentTarget.value)}
            />
            <Show when={fieldErrors().cancelCutoffMinutes}>
              {(msg) => (
                <div
                  role="alert"
                  class="error field-error"
                  data-error-for="cancelCutoffMinutes"
                  data-field-error="cancelCutoffMinutes"
                >
                  {msg()}
                </div>
              )}
            </Show>
          </div>
          <div class="form-group">
            <label for="noShowGraceMinutes">{t('noShowGraceMinutes')}</label>
            <input
              id="noShowGraceMinutes"
              name="noShowGraceMinutes"
              data-field="noShowGraceMinutes"
              type="number"
              min="0"
              max="129600"
              aria-invalid={fieldErrors().noShowGraceMinutes ? 'true' : undefined}
              value={grace()}
              onInput={(e) => setGrace(e.currentTarget.value)}
            />
            <Show when={fieldErrors().noShowGraceMinutes}>
              {(msg) => (
                <div
                  role="alert"
                  class="error field-error"
                  data-error-for="noShowGraceMinutes"
                  data-field-error="noShowGraceMinutes"
                >
                  {msg()}
                </div>
              )}
            </Show>
          </div>
          <div class="form-group">
            <label for="confirmExpiryMinutes">{t('confirmExpiryMinutes')}</label>
            <input
              id="confirmExpiryMinutes"
              name="confirmExpiryMinutes"
              data-field="confirmExpiryMinutes"
              type="number"
              min="1"
              max="129600"
              aria-invalid={fieldErrors().confirmExpiryMinutes ? 'true' : undefined}
              value={expiry()}
              onInput={(e) => setExpiry(e.currentTarget.value)}
            />
            <Show when={fieldErrors().confirmExpiryMinutes}>
              {(msg) => (
                <div
                  role="alert"
                  class="error field-error"
                  data-error-for="confirmExpiryMinutes"
                  data-field-error="confirmExpiryMinutes"
                >
                  {msg()}
                </div>
              )}
            </Show>
          </div>
          <button type="button" disabled={writing()} onClick={handleSavePolicy}>
            {t('savePolicy')}
          </button>
        </section>
      </Show>
    </div>
  );
}
