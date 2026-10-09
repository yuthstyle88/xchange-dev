# Blueprint languages: TH / EN / ZH on customer screens, TH / EN on staff screens (#576)

> designed-by: claude/b060e9 · 2026-10-09 · measured on `main` at `1bf3323`
> Source of the rule: *XC Desk — Blueprint & Data Model v2* §15 (screens) and §18 acceptance
> *"TH/EN/ZH labels และ placeholders ครบ; date/currency locale ไม่เปลี่ยน stored amount"*, plus P01 *"เลือกภาษา"*.
> Builds on: `docs/design/xchange-v3-web.md` W10 (the shared i18n module, one table per screen folder).
> Umbrella: #564. The owner's standing rule is *"logic ห้ามแก้ไข — ปรับได้แค่หน้าตา"*. A language changes only the
> look, never an amount or a state (§15: *"locale เปลี่ยนการแสดงผล ไม่เปลี่ยนจำนวนหรือสถานะธุรกรรม"*).

## 0. The owner's answers (2026-10-09, typed in session claude/b060e9, recorded on #576 issuecomment-6074348040 and issuecomment-6074443343)

| # | Question | Answer |
|---|---|---|
| L-A1 | Which Chinese | **Simplified, `zh-CN`** |
| L-A2 | Who supplies the Chinese text | **Machine draft + review**: the ticket author drafts every `zh` string, and a person who reads Chinese checks the list before the screen ships |
| L-A3 | Which screens first | first answer: customer-facing first, then staff. **Superseded by L-A4** |
| L-A4 | Which screens speak Chinese | *"Chinese (zh-CN) for customer-facing screens only. Admin/back-office screens stay TH/EN — no Chinese."* |

## 1. v3 today (measured at `1bf3323`)

| Fact | Evidence |
|---|---|
| The language type has two members | `web/src/shared/i18n/index.ts:5` `Lang = 'th' \| 'en'`, `:6` `Locale = 'th-TH' \| 'en-GB'` |
| A stored value other than `th`/`en` reads as Thai | `index.ts:17` |
| `<html lang>` is the bare language | `index.ts:25-33` (`syncHtmlLang`) |
| Lookup picks between two tables in four places | `t` (`:65-70`), `translator` (`:173-188`), `screenT` (`:190-213`), `locale` (`:41`) |
| A screen table must export `th` and `en`, with equal keys and placeholders, or the page throws at load | `collectScreens` `:89-163` |
| The switch is a two-way toggle | `LangSwitch.tsx:7-11` |
| **20 screen tables** on `main`, about 800 Thai keys in all | `find web/src/screens -name i18n.ts`. Biggest: `admin/cash` (~93), `admin/customers` (~77), `admin/reports` (~70) |
| One place builds a screen table by hand, outside `i18n.ts` | `screens/devices/labels.ts:41-63` (`labelsTh`/`labelsEn` from `STATUS_LABELS`/`KIND_LABELS`, plus an unused `_lang?: 'th' \| 'en'` parameter) |
| Dates follow `locale()` in 10 files, and `Intl.DisplayNames` follows `lang()` in one | `grep -rn "locale()\|lang()" web/src` (non-test): `QueuePanel.tsx:56`, `admin/cash/Cash.tsx:32`, `admin/rates/Rates.tsx:23`, … |
| Server HTML (receipt, slip sample) is Thai only | `api/src/presentation/http/documents.rs:155`, `:229`, `api/src/presentation/pages.rs:382` (`<html lang="th">`) |
| The kiosk (Flutter, `app/`) already speaks 16 languages, Chinese included | `app/assets/l10n/kiosk.json`; it is outside this design |

## 2. Decisions

| # | Decision | Source |
|---|---|---|
| L1 | `Lang` becomes `'th' \| 'en' \| 'zh'`. `Locale` becomes `'th-TH' \| 'en-GB' \| 'zh-CN'`. `<html lang>` becomes `zh-CN` for Chinese. | §15, L-A1 |
| L2 | **Two audiences.** A **customer** screen speaks TH/EN/ZH. A **staff** screen (desk, staff, admin, devices, set-up, login, guides) speaks TH/EN only and **never** carries `zh`. | L-A4 |
| L3 | **The audience is the folder.** Customer screens live under `web/src/screens/customer/**`, and every other folder is staff. The core reads the audience from the glob path, so no screen declares which tables must carry `zh` and no shared list grows. (The runtime audience is set by the customer entry, L5.) | W10 point 1 (one file per folder, no shared table) |
| L4 | **A customer table must export a complete `zh`**: same keys and same placeholders as `th`, with no empty value. **A staff table must not export `zh`.** Either breach throws at page load, naming the screen, like a missing `en` key today. | L2, §18 *"ครบ"* |
| L5 | **The switch offers what the page's audience speaks.** A customer page's entry module calls `setAudience('customer')` before it renders, and `LangSwitch` shows `TH / EN / 中文` or `TH / EN`. `setLang` ignores a language the current audience does not speak. A stored `xc.lang = 'zh'` opened on a staff page shows Thai (the default) and is not overwritten, so the customer's choice survives. | L2 |
| L6 | **Per-key fallback is `zh → en → key`** on customer pages: it guards a shared key the shared `zh.ts` lacks, never Thai and never `''`. Staff pages keep today's `th`/`en` lookup unchanged. | #576 §3 |
| L7 | **The Chinese is reviewed before its screen merges.** The ticket author drafts every `zh` string (L-A2). A person who reads Chinese checks the draft on that ticket, and the reviewer of the PR checks that the review is recorded there. No global switch-on flag is needed. | L-A2 |
| L8 | Dates and currency names follow `locale()` / `lang()` as today, so they become Chinese with no screen change. `MoneyText` digits do not change (W7, §18 *"ไม่เปลี่ยน stored amount"*). | §15, §18 |
| L9 | Server refusals: the shared `zh.ts` carries `error.<code>` for every code the shared `th.ts` carries. `errorText` on a customer page uses `zh → en → server message`. The server still sends Thai (W10 point 4). | W10 point 4 |
| L10 | **Out of this design, named for their owners:** the language of server HTML documents a customer receives (receipt) follows the customer's `preferred_language` (Blueprint §07 Customer), which the documents design and #554 (customers) own. Notification templates carry `locale` (§16 Notification) and belong to #558. The Flutter kiosk keeps its own 16 languages. | §07, §16 |

**The existing 20 screen tables are all staff screens** (§1). None of them changes, and no ticket here touches them.

## 3. Model

### 3.1 Types and files

```ts
// web/src/shared/i18n/languages.ts   (new, LN-1)   [required names]
export const LANGS = ['th', 'en', 'zh'] as const;
export type Lang = (typeof LANGS)[number];
export type Audience = 'customer' | 'staff';
export const SPOKEN: Record<Audience, readonly Lang[]> = { customer: ['th', 'en', 'zh'], staff: ['th', 'en'] };
export const CUSTOMER_ROOT = 'customer';                    // first folder segment under web/src/screens
export const LOCALE: Record<Lang, 'th-TH' | 'en-GB' | 'zh-CN'> = { th: 'th-TH', en: 'en-GB', zh: 'zh-CN' };
export const HTML_LANG: Record<Lang, string> = { th: 'th', en: 'en', zh: 'zh-CN' };
export const NATIVE_NAME: Record<Lang, string> = { th: 'ไทย', en: 'English', zh: '中文' };
```

```ts
// web/src/shared/i18n/index.ts   (LN-1)   [required: exported names and signatures; body proposed]
export type { Lang, Audience } from './languages';
export type Locale = (typeof LOCALE)[Lang];
export function setAudience(a: Audience): void;            // called once by a customer page's entry module, before render
// The audience starts as 'staff' [required]: a page that never calls setAudience('customer') (every staff page, login, AuthLayout pages)
// is TH/EN. setAudience('customer') is the only way to get Chinese.
export function setLang(l: Lang): void;                    // ignored when l is not in SPOKEN[audience]
export const lang: Accessor<Lang>;                          // the stored choice if the audience speaks it, else 'th'
export const locale: Accessor<Locale>;
export function t(key: Key, params?: Params): string;
export function screenT<T extends ScreenTable>(screen: string, table: T)
  : (key: keyof T['th'] & string, params?: Params) => string;
export interface ScreenTable { readonly th: Record<string,string>; readonly en: Record<string,string>; readonly zh?: Record<string,string> }
export interface ScreenTables { th: Record<string,string>; en: Record<string,string>; zh: Record<string,string> }
export function collectScreens(glob: Record<string, unknown>): ScreenTables;
```

```ts
// web/src/shared/i18n/zh.ts   (new, LN-1)   [required]
import type { Key } from './th';
export const zh: Record<Key, string> = { /* every shared key: field.*, form.*, shell.*, error.*, queue.* */ };
```

```ts
// web/src/screens/customer/<path>/i18n.ts   (every customer screen)   [required shape]
export const th = { … } as const;
export const en: Record<keyof typeof th, string> = { … };
export const zh: Record<keyof typeof th, string> = { … };   // complete
// web/src/screens/<any other folder>/i18n.ts keeps today's th + en, and exports no zh
```

**No `AppShell` change.** Since the audience starts as `'staff'` (§3.1), a staff page needs no call and no prop. A customer page's
entry module (`web/src/screens/customer/<x>/main.tsx`) calls `setAudience('customer')` once before `render`. [required]
This replaces an earlier `AppShell` `audience` prop: that prop would have made LN-1 edit `AppShell.tsx` and every screen
that renders it, the same files the look tickets LK-1/2/4/5 (#630–#634) hold. (Amended 2026-10-09 by the designer, claude/b060e9.)

### 3.2 Rules the core enforces (`collectScreens`, at page load, as today)

| Rule | Result on failure |
|---|---|
| `th` and `en` present, same keys and placeholders (today's rule) | throws, unchanged |
| a table under `screens/customer/**` without `zh` | throws `Screen "<s>" is a customer screen and must export zh` |
| a customer `zh` missing a key, with an extra key, with a renamed placeholder, or with an empty value | throws, naming the screen and key |
| a table outside `screens/customer/**` that exports `zh` | throws `Screen "<s>" is a staff screen: TH/EN only` |
| `customer` used as a non-first segment (`admin/customer/…`) | allowed: only the first segment decides (today's segment regex unchanged) |

### 3.3 The switch

**Staff pages keep today's switch exactly.** That is one toggle button showing the *other* language (`EN` on a Thai page, `TH`
on an English one), labelled `t('shell.langSwitch', { language: … })` (`LangSwitch.tsx:5-12` on `main`). Existing tests
pin it (`AuthLayout.test.tsx:158-163`, `AppShell.lock.test.tsx:256`, `AdminShell.test.tsx`, `i18n.test.ts`), and LN-1
changes none of them. **Customer pages** (audience `customer`, three languages) render one button per language in
`SPOKEN.customer` (`TH`, `EN`, `中文`). The current one is marked `aria-pressed="true"`, and each is labelled
`t('shell.langSwitch', { language: NATIVE_NAME[l] })`. [required: the staff toggle unchanged, and only the
audience's languages on customer pages. proposed: buttons; a `<select>` is fine if the customer page design prefers it.]
(Amended 2026-10-09 by the designer, claude/b060e9.)

## 4. Class diagram

```mermaid
classDiagram
  class Languages { <<web/src/shared/i18n/languages.ts>> +LANGS +SPOKEN +CUSTOMER_ROOT +LOCALE +HTML_LANG +NATIVE_NAME }
  class I18nCore { <<web/src/shared/i18n/index.ts>> +lang() +locale() +setAudience(a) +setLang(l) +t(key) +screenT(screen, table) +collectScreens(glob) }
  class SharedTh { <<web/src/shared/i18n/th.ts>> }
  class SharedEn { <<web/src/shared/i18n/en.ts>> }
  class SharedZh { <<web/src/shared/i18n/zh.ts  new>> }
  class LangSwitch { <<web/src/shared/i18n/LangSwitch.tsx>> }
  class CustomerEntry { <<web/src/screens/customer/*/main.tsx>> }
  class CustomerTable { <<web/src/screens/customer/**/i18n.ts>> th; en; zh }
  class StaffTable { <<web/src/screens/(other)/**/i18n.ts>> th; en }

  I18nCore --> Languages
  I18nCore --> SharedTh
  I18nCore --> SharedEn
  I18nCore --> SharedZh
  I18nCore ..> CustomerTable : import.meta.glob
  I18nCore ..> StaffTable : import.meta.glob
  CustomerEntry --> I18nCore : setAudience('customer')
  LangSwitch --> Languages : SPOKEN, NATIVE_NAME
  LangSwitch --> I18nCore : setLang, t
```

### 4.1 Who provides, who uses

| Provider | Used by | Fallback if not merged |
|---|---|---|
| LN-1 `Lang`, `Audience`, `screens/customer/**` rule, shared `zh.ts` | every customer screen ticket (rate board P01 #548/#603, the member/booking web parts of #558, any other `customer/` screen) | none: a customer screen ticket starts after LN-1. Before it, a `zh` export would be ignored and untyped |
| The Chinese reviewer (L-A2, L7) | each customer screen ticket | that ticket's PR waits |

## 5. Data migration

None. The language is a browser preference (`xc.lang` in `localStorage`, Blueprint §17 *"language preference
localStorage ได้"*). No table, column or stored value changes. A stored `zh` opened on a staff page shows Thai and is
left in storage (L5). Customer `preferred_language` (§07) is #554's column, with its own migration section there.

## 6. Tickets

| # | Ticket | Owns | Starts after |
|---|---|---|---|
| LN-1 | core: `Lang`/`Locale` with `zh`, `languages.ts` (`Audience`, `SPOKEN`, `CUSTOMER_ROOT`), `setAudience` (audience starts `'staff'`), the audience rules of §3.2 in `collectScreens`, fallback `zh → en → key` on customer pages, `HTML_LANG`, shared `zh.ts`, `LangSwitch` from `SPOKEN` | `web/src/shared/i18n/{index.ts,languages.ts,zh.ts,LangSwitch.tsx}` and a new test file of its own. **No other file**: no `AppShell`, no screen, no existing test (the existing i18n tests stay green unchanged, since staff behaviour does not change) | this design |

**Customer screens.** Every customer screen ticket cut after LN-1 merges puts its folder under
`web/src/screens/customer/`, ships `th`, `en` and a complete `zh` in its own `i18n.ts`, and its `main.tsx` calls
`setAudience('customer')` before `render`. Its ticket author drafts the `zh` strings in the ticket body (`key | th | en | zh`). A person
who reads Chinese marks the draft checked on that ticket before the PR is reviewed (L7). The implementer copies the
draft unchanged. Staff screen tickets (the Wave 3 desk, staff, staff report and documents screens included) stay
TH/EN and carry no `zh`. Their designs' note "zh once #576 adds it" is superseded by L2 and should be removed in their
next edit.

### 6.1 Tests written before implementation (LN-1)

- **Staff pages.** `setLang('zh')` changes nothing. A stored `xc.lang = 'zh'` shows Thai and stays in storage. The switch is still the single toggle (`EN`, then `TH` after a click).
- **No audience set.** With no `setAudience` call at all, the audience is `staff`. `LangSwitch` is the single toggle, and `setLang('zh')` does nothing. This keeps `LoginScreen.tsx` and `AuthLayout.tsx` untouched.
- **Customer pages, with a fixture table under `customer/`.**
  - `setLang('zh')` gives `lang()` `zh`, `locale()` `zh-CN` and `<html lang>` `zh-CN`.
  - The screen shows its `zh` text.
  - A shared key missing from a stubbed `zh.ts` shows `en`, never `th` and never `''`.
  - The switch shows three buttons, each labelled through `t`.
- **Collect errors.** Each of these throws at collect, naming the screen and key:
  - a customer table without `zh`;
  - a customer `zh` missing one key, with one extra key, with a renamed placeholder, or with an empty value;
  - a staff table that exports `zh`.
- **Shared table.** `zh.ts` has every key of `th.ts`, no empty value and the same placeholders, plus an `error.<code>` for every `error.*` key.
- **Errors in Chinese.** `errorText` on a customer page in Chinese shows the table text, else the server's message.
- **No `AppShell` change.** A customer fixture page that calls `setAudience('customer')` gets three buttons from the `LangSwitch` that `AppShell` already renders.
- **Every customer entry declares its audience before it renders.** This is the regression guard that the required `AppShell` prop gave in PR 607.
  - The test reads every `web/src/screens/customer/*/main.tsx` through `import.meta.glob(..., { query: '?raw', eager: true })`.
  - It asserts that each file calls `setAudience('customer')`, and that the call comes before its first `render(`.
  - It passes vacuously until the first customer screen lands. From then on, an entry that forgets the call, or makes it after `render`, fails.
  - (Added 2026-10-09 on claude/9e7ea4c7's review of PR 666.)

## 7. CI gates that measure this ticket

- `web.yml` runs typecheck, vitest and build. Typing `zh` as `Record<keyof typeof th, string>` makes a missing key a compile error in every customer folder.
- The ratchet on `any` is not touched, because no table needs an untyped value.

## 8. Must ask the owner

None open. L-A1…L-A4 are answered (§0).

## 9. Blueprint conformance

| Blueprint item | This design |
|---|---|
| §15 *"ภาษาเสนอ TH/EN/ZH ใช้ translation keys ฝั่ง UI"* | L1–L4, on customer screens. Staff screens are TH/EN by the owner's decision (L-A4) |
| §15 *"locale เปลี่ยนการแสดงผล ไม่เปลี่ยนจำนวนหรือสถานะธุรกรรม"* | L8: only text, dates and currency names change |
| §18 *"TH/EN/ZH labels และ placeholders ครบ"* | L4 parity on every customer table; L-A4 limits it to customer screens |
| §18 *"date/currency locale ไม่เปลี่ยน stored amount"* | L8, W7 (`MoneyText` digits unchanged) |
| P01 *"เลือกภาษา"* on the customer rate screen | L5: a customer page's switch shows TH / EN / 中文 |
| §17 *"language preference localStorage ได้"* | §5 |
| §07 Customer `preferred_language`, §16 Notification `locale` | L10, owned by #554, the documents design and #558 |
