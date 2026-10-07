// Server refusals that name exactly one field, so the screen can point at it (row FV).

const BRANCH = 'ไม่พบสาขาที่เลือก กรุณารีเฟรช';
const PASSWORD = 'รหัสผ่านต้องมี 12–128 ตัวอักษร'; // U+2013 en-dash

/** error element id → (refusal sentence → id of the field it names). */
export const REFUSALS: Readonly<Record<string, Readonly<Record<string, string>>>> = Object.freeze({
  'employee-error': Object.freeze({
    [BRANCH]: 'employee-branch',
    [PASSWORD]: 'employee-password',
  }),
  'reset-error': Object.freeze({
    [PASSWORD]: 'reset-employee-password',
  }),
});

/** The id of the field a refusal shown in `errorId` names, or null when it names none. */
export function fieldFor(errorId: unknown, text: unknown): string | null {
  if (typeof errorId !== 'string' || typeof text !== 'string') return null;
  if (!Object.prototype.hasOwnProperty.call(REFUSALS, errorId)) return null;
  const mapping = REFUSALS[errorId];
  const sentence = text.trim();
  return Object.prototype.hasOwnProperty.call(mapping, sentence) ? mapping[sentence] : null;
}
