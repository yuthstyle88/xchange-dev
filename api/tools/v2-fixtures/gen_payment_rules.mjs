// Records the payment rules of the 10-06 bundle (design §5.5.2, row PM-R): the five channels and
// their defaults, which channel a method name and a direction are allowed on, and which sets of
// five rows the bundle accepts.
//   node gen_payment_rules.mjs <10-06 bundle dir> > ../../crates/xc-kernel/tests/fixtures/v2_payment_rules_cases.json
//
// It needs the 10-06 bundle (`XC-Desk-v2-Passport-Drawer-20261006.zip`, sha256 6514b4c8…4375f570;
// `server.mjs` sha256 99ddcd10…8a1b0b): its `payment-settings.mjs` is imported from the directory
// given, unchanged. The bundle is not in this repository, so this cannot run in CI; its output is
// checked in and is what the tests read.
//
// Only the module's pure exports are called — `METHODS`, `defaults`, `validateMethods`, `allowed` —
// never `createPaymentSettings` (no database, no clock). Nothing of the module is copied here.
//
// The script fails, and writes nothing, when the module answers a sentence that is not in the list
// below, or when a sentence of the list was never heard.
import fs from 'node:fs';
import path from 'node:path';
import { pathToFileURL } from 'node:url';

const dir = process.argv[2];
if (!dir) throw Error('usage: node gen_payment_rules.mjs <10-06 bundle dir>');
const file = path.join(fs.statSync(dir).isDirectory() ? dir : path.dirname(dir), 'payment-settings.mjs');
const v2 = await import(pathToFileURL(file).href);
for (const name of ['METHODS', 'defaults', 'validateMethods', 'allowed']) if (!(name in v2)) throw Error('payment-settings.mjs: ' + name + ' is not exported');

// The sentences `validateMethods` may answer with (design §5.5.2 rows 9, 10, 11, 12, 14). Row 10
// is heard here only for five distinct ids one of which is not a channel (the channel it replaced
// is then missing); a value that is not a boolean, and row 13 (a bad text), cannot be written as a
// `ChannelRule` and are recorded for the parser instead (gen_payment_methods.mjs).
const SENTENCES = {
  notFive: 'กรุณาตั้งค่าให้ครบทั้ง 5 ช่องทาง',
  channelMissing: 'ข้อมูลช่องทางชำระเงินไม่ถูกต้อง',
  buyOnlyCashOrTransfer: 'การจ่ายเงินบาทให้ลูกค้ารองรับเงินสดและโอนธนาคาร',
  onWithNoDirection: 'ช่องทางที่เปิดต้องเลือกการรับเงินหรือจ่ายเงินอย่างน้อยหนึ่งประเภท',
  oneLeftPerDirection: 'ต้องเหลือช่องทางรับเงินและจ่ายเงินบาทอย่างน้อยประเภทละ 1 ช่องทาง',
};
const heard = new Set();
const known = new Map(Object.entries(SENTENCES).map(([k, v]) => [v, k]));

const METHODS = [...v2.METHODS];
const base = () => v2.defaults();
const edit = (rows, id, change) => rows.map((r) => (r.id === id ? { ...r, ...change } : r));
const only = (rows, ids) => rows.map((r) => ({ ...r, enabled: ids.includes(r.id) }));

// ── sets of rows: each is given to validateMethods; what it answers is recorded ──
const sets = [];
const add = (label, rows) => sets.push({ label, rows: structuredClone(rows) });
add('the defaults', base());
add('the defaults in reverse order', base().reverse());
add('the defaults with texts', edit(edit(base(), 'Cash', { account: 'Drawer 1', instruction: 'Pay at the counter' }), 'PromptPay', { account: 'PP 000', instruction: 'Scan the code' }));
add('everything off but Cash', only(base(), ['Cash']));
add('everything off but Bank Transfer', only(base(), ['Bank Transfer']));
add('Cash for SELL only, Bank Transfer for BUY only', edit(edit(only(base(), ['Cash', 'Bank Transfer']), 'Cash', { buy: false }), 'Bank Transfer', { sell: false }));
add('Credit Card off but still marked for SELL', edit(base(), 'Credit Card', { enabled: false }));
add('a channel off with neither direction', edit(base(), 'Alipay', { enabled: false, buy: false, sell: false }));
add('BUY on Credit Card', edit(base(), 'Credit Card', { buy: true }));
add('BUY on Alipay while Alipay is off', edit(base(), 'Alipay', { enabled: false, buy: true }));
add('BUY on PromptPay', edit(base(), 'PromptPay', { buy: true }));
add('Cash on with neither direction', edit(base(), 'Cash', { buy: false, sell: false }));
add('PromptPay on with neither direction', edit(base(), 'PromptPay', { buy: false, sell: false }));
add('no channel on for BUY', edit(edit(base(), 'Cash', { buy: false }), 'Bank Transfer', { buy: false }));
add('BUY only on channels that are off', edit(edit(base(), 'Cash', { enabled: false }), 'Bank Transfer', { enabled: false }));
add('no channel on for SELL', ['Cash', 'Credit Card', 'Alipay', 'PromptPay', 'Bank Transfer'].reduce((rows, id) => edit(rows, id, { sell: false }), only(base(), ['Cash', 'Bank Transfer'])));
add('every channel off', only(base(), []));
add('four rows', base().slice(0, 4));
add('six rows', [...base(), { ...base()[0] }]);
add('Cash twice, Bank Transfer missing', [...base().slice(0, 4), { ...base()[0] }]);
add('an unknown channel instead of Alipay', edit(base(), 'Alipay', { id: 'Cheque' }));
add('SCB Transfer instead of Bank Transfer', edit(base(), 'Bank Transfer', { id: 'SCB Transfer' }));

const fromRows = sets.map(({ label, rows }) => {
  try {
    return { label, rows, accepted: v2.validateMethods(structuredClone(rows)) };
  } catch (e) {
    if (!known.has(e.message)) throw Error('validateMethods answered a sentence this script does not know: ' + e.message + ' (' + label + ')');
    heard.add(e.message);
    return { label, rows, refused: e.message };
  }
});

// ── which method and direction each set allows ──
const NAMES = [...METHODS, 'SCB Transfer', 'Cheque', '', 'cash', 'Bank transfer'];
const allows = fromRows.filter((c) => c.accepted).map(({ label, accepted }) => ({
  label,
  rows: accepted,
  answers: NAMES.flatMap((method) => ['BUY', 'SELL'].map((type) => ({ method, type, allowed: v2.allowed(accepted, method, type) }))),
}));

for (const s of Object.values(SENTENCES)) if (!heard.has(s)) throw Error('never heard: ' + s);

process.stdout.write(JSON.stringify({
  source: 'payment-settings.mjs of the 10-06 bundle (XC-Desk-v2-Passport-Drawer-20261006.zip, sha256 6514b4c8…4375f570), through gen_payment_rules.mjs',
  methods: METHODS,
  defaults: base(),
  fromRows,
  allows,
}, null, 1) + '\n');
