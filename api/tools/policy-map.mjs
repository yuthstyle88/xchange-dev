#!/usr/bin/env node
// Re-expresses v2's policy fixture in the Blueprint's roles (#550 OR-4,
// docs/design/blueprint/org-roles-branches.md §4.3).
//
//   node api/tools/policy-map.mjs          writes   api/crates/xc-auth/tests/fixtures/blueprint_policy_cases.json
//   node api/tools/policy-map.mjs --check  exits 1 when the committed file is not this script's output
//
// What it does, and nothing else:
// - every caller becomes the B1-mapped user (§3.3; OWNER is one user holding EXECUTIVE + ADMIN, B3);
// - every answer stays v2's, except where a rule below (C1–C9 of §4.2, D4 of §2) names the case;
// - the case count stays 1,425 (and 285 branch cases), in v2's order, so case i here is case i there;
// - the custody rows (path Custody, the staff bridge's `ops…` names on StaffRpc, and adminOpenShift /
//   adminCloseShift on both RPC paths) are #582's:
//   they keep v2's answer and carry `"owner": "#582"`; OR-4 does not replay them.
//
// A main case named B9 for an employee of B2; v2 ignored the name and ran on B2. The mapped case names
// the branch v2 ran it on (`named` = `own`), so that only the branch cases test branch scope (D4).

import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const FIXTURES = path.join(here, '..', 'crates', 'xc-auth', 'tests', 'fixtures');
const V2 = path.join(FIXTURES, 'v2_policy_cases.json');
const OUT = path.join(FIXTURES, 'blueprint_policy_cases.json');

// §3.3 (B1, B3). `null` branch = the whole organisation.
export const B1 = {
  OWNER: [
    { role: 'EXECUTIVE', branch: null },
    { role: 'ADMIN', branch: null },
  ],
  FRONT: [{ role: 'TELLER', branch: 'own' }],
  BACKEND: [{ role: 'BRANCH_MANAGER', branch: 'own' }],
  MANAGER: [{ role: 'BRANCH_MANAGER', branch: 'own' }],
  RATES: [{ role: 'RATE_MANAGER', branch: 'own' }],
  FINANCE: [{ role: 'FINANCE', branch: 'own' }],
  SYSTEM: [{ role: 'ADMIN', branch: 'own' }],
  MONITOR: [{ role: 'VERIFICATION_REVIEWER', branch: 'own' }],
  DOCUMENTS: [{ role: 'VERIFICATION_REVIEWER', branch: 'own' }],
  DOCUMENT_REVIEW: [{ role: 'VERIFICATION_REVIEWER', branch: 'own' }],
};

export const OUT_OF_SCOPE = { error: 'สาขานี้อยู่นอกขอบเขตของบัญชีนี้', status: 403 };

const STAFF_PATHS = ['StaffRpc', 'StaffUpload', 'StaffFile', 'StaffDesk'];
const KIOSK_QUEUE = ['identityChecked', 'publishReceipt', 'get', 'accept'];

const allowed = () => ({ allowed: true });
const refused = (sentence, status) => ({ allowed: false, error: sentence, status });

// Each rule: id, the design's words, which v2 cases it selects, and what they become.
// `to(case, v2)` returns the new outcome; `v2` is the whole v2 fixture (C1 reads SYSTEM's row).
export const RULES = [
  {
    id: 'C1',
    says: "§4.2 C1 (§07 one User): a staff path admits whoever holds its permission; the owner's user is answered as its roles are (EXECUTIVE + ADMIN), and where neither holds the row's permission it hears what ADMIN (ex-SYSTEM) hears",
    selects: (c) => STAFF_PATHS.includes(c.path) && c.actor === 'OWNER',
    to: (c, v2) => {
      // rows whose permission EXECUTIVE or ADMIN holds: CompatRead (A), RatesDraft (E), ProofUpload (E)
      const passes =
        (c.path === 'StaffRpc' && ['adminSnapshot', 'getConnectionStatus', 'adminSaveRate'].includes(c.action)) ||
        c.path === 'StaffUpload';
      if (passes) return allowed();
      const sys = v2.cases.find((s) => s.path === c.path && s.action === c.action && s.actor === 'SYSTEM');
      return outcome(sys);
    },
  },
  {
    id: 'C2',
    says: '§4.2 C2: reserveCustomer and saveTransaction on Rpc are ExchangeTransact, TELLER only; EXECUTIVE is refused (§03 B03)',
    selects: (c) => c.path === 'Rpc' && ['reserveCustomer', 'saveTransaction'].includes(c.action) && c.actor === 'OWNER',
    to: () => refused('บทบาทนี้ไม่มีสิทธิ์ทำรายการนี้', 400),
  },
  {
    id: 'C3',
    says: '§4.2 C3: adminMoveStock is CashManage, BRANCH_MANAGER and FINANCE; EXECUTIVE is refused, BRANCH_MANAGER allowed (§03 B05)',
    selects: (c) => c.path === 'Rpc' && c.action === 'adminMoveStock' && ['OWNER', 'BACKEND', 'MANAGER'].includes(c.actor),
    to: (c) => (c.actor === 'OWNER' ? refused('บทบาทนี้ไม่มีสิทธิ์ทำรายการนี้', 400) : allowed()),
  },
  {
    id: 'C4',
    says: '§4.2 C4: branches and employees are BranchesManage / UsersManage, ADMIN; ADMIN (ex-SYSTEM) is allowed, the owner keeps them through ADMIN (§03 A04/A05)',
    selects: (c) =>
      c.actor === 'SYSTEM' &&
      ((c.path === 'Rpc' && c.action === 'adminCreateBranch') || (c.path === 'OwnerRoute' && c.route === 'POST /api/employees')),
    to: () => allowed(),
  },
  {
    id: 'C5',
    says: '§4.2 C5: payment settings are SystemSettings, EXECUTIVE and ADMIN; ADMIN (ex-SYSTEM) is allowed (§02 ADMIN "ตั้งค่าระบบ")',
    selects: (c) => c.path === 'PaymentMethods' && c.action === 'save' && c.actor === 'SYSTEM',
    to: () => allowed(),
  },
  {
    id: 'C7',
    says: '§4.2 C7: B1 merges BACKEND and MANAGER into BRANCH_MANAGER, so ex-MANAGER gains process, retry, drawerObservation and printerOnline',
    selects: (c) =>
      c.path === 'Devices' &&
      ['process', 'retry', 'drawerObservation', 'printerOnline'].includes(c.action) &&
      c.actor === 'MANAGER',
    to: () => allowed(),
  },
  {
    id: 'C8',
    says: "§4.2 C8: the kiosk queue is QueueOperate, TELLER and BRANCH_MANAGER; EXECUTIVE is refused (§03 B06). A name the kiosk does not know is 'Unknown kiosk action.' only to a QueueOperate holder, so the owner's moves with it",
    selects: (c) => c.path === 'Kiosk' && [...KIOSK_QUEUE, 'nope'].includes(c.action) && c.actor === 'OWNER',
    to: () => refused('Staff access required.', 400),
  },
];

// D-4 (§2, §3.5): a branch outside the caller's scope is refused, no longer replaced. Branch cases only.
export const D4 = {
  id: 'D4',
  says: "§2 D-4 / §3.5: an employee naming a branch outside their scope is refused 'outOfScope'; a public kiosk row has no scope and runs on the branch named",
  selects: (c) => c.own != null && c.named != null && c.named !== c.own,
  to: (c) => (c.path === 'Kiosk' && c.action === 'rates' ? { allowed: true, branch: c.named } : refused(OUT_OF_SCOPE.error, OUT_OF_SCOPE.status)),
};

// C6 (MARKETING) and C9 select no v2 case: no v3 user is MARKETING, and C9 is not
// in v2's fixture. They are asserted by the Rust tests that name them.
export const NO_CASE = ['C6', 'C9'];

// #582's rows (§4.2 "Custody ops*" and "Rpc adminOpenShift, adminCloseShift" → #582, PR 590): kept as v2 answered.
export const isCustody = (c) =>
  c.path === 'Custody' ||
  (c.path === 'StaffRpc' && c.action.startsWith('ops')) ||
  (['Rpc', 'StaffRpc'].includes(c.path) && ['adminOpenShift', 'adminCloseShift'].includes(c.action));

function outcome(c) {
  const o = { allowed: c.allowed };
  for (const k of ['branch', 'error', 'status', 'redirect', 'behind']) if (c[k] !== undefined) o[k] = c[k];
  return o;
}

function roles(actor, own) {
  if (actor === 'ANON') return null;
  return B1[actor].map((r) => ({ role: r.role, branch: r.branch === 'own' ? own : null }));
}

function mapCase(c, v2, rules, isBranchCase) {
  const out = { path: c.path, route: c.route, actor: c.actor, roles: roles(c.actor, c.own), action: c.action };
  out.own = c.own ?? null;
  out.named = isBranchCase ? (c.named ?? null) : c.own != null ? c.own : (c.named ?? null);
  let next = outcome(c);
  if (isCustody(c)) {
    out.owner = '#582';
  } else {
    for (const r of rules) {
      if (r.selects(c)) {
        next = r.to(c, v2);
        out.change = r.id;
        break;
      }
    }
  }
  return { ...out, ...next };
}

export function build(v2) {
  const cases = v2.cases.map((c) => mapCase(c, v2, RULES, false));
  const branchCases = v2.branchCases.map((c) => mapCase(c, v2, [D4], true));
  const sentences = new Map();
  for (const c of [...cases, ...branchCases]) if (!c.allowed && c.error) sentences.set(c.error + '\u0000' + c.status, { error: c.error, status: c.status });
  for (const c of cases) if (c.behind) sentences.set(c.behind.error + '\u0000' + c.behind.status, c.behind);
  return {
    source: {
      from: 'v2_policy_cases.json',
      by: 'api/tools/policy-map.mjs',
      design: 'docs/design/blueprint/org-roles-branches.md §3.3, §4.2, §4.3',
    },
    counts: {
      cases: cases.length,
      allowed: cases.filter((c) => c.allowed).length,
      refused: cases.filter((c) => !c.allowed).length,
      branchCases: branchCases.length,
      custody: cases.filter((c) => c.owner === '#582').length,
      changed: cases.filter((c) => c.change).length,
      branchChanged: branchCases.filter((c) => c.change).length,
    },
    roles: ['EXECUTIVE', 'ADMIN', 'RATE_MANAGER', 'BRANCH_MANAGER', 'TELLER', 'FINANCE', 'MARKETING', 'CUSTOMER', 'VERIFICATION_REVIEWER'],
    b1: B1,
    rules: [...RULES, D4].map((r) => ({ id: r.id, says: r.says })),
    noCase: NO_CASE,
    outOfScope: OUT_OF_SCOPE,
    refusals: [...sentences.values()].sort((a, b) => (a.error < b.error ? -1 : a.error > b.error ? 1 : 0)),
    cases,
    branchCases,
  };
}

// One case per line, so a diff of this file reads case by case.
export function render(v2) {
  const all = build(v2);
  const { cases, branchCases, ...head } = all;
  const lines = (xs) => '[\n' + xs.map((x) => '  ' + JSON.stringify(x)).join(',\n') + '\n ]';
  const top = JSON.stringify(head, null, 1).replace(/\n}$/, '');
  return `${top},\n "cases": ${lines(cases)},\n "branchCases": ${lines(branchCases)}\n}\n`;
}

if (process.argv[1] && fileURLToPath(import.meta.url) === path.resolve(process.argv[1])) {
  const text = render(JSON.parse(fs.readFileSync(V2, 'utf8')));
  if (process.argv.includes('--check')) {
    const committed = fs.existsSync(OUT) ? fs.readFileSync(OUT, 'utf8') : '';
    if (committed !== text) {
      console.error(`${path.relative(process.cwd(), OUT)} is not the output of policy-map.mjs; run: node api/tools/policy-map.mjs`);
      process.exit(1);
    }
  } else {
    fs.writeFileSync(OUT, text);
  }
}
