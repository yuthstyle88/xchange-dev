import * as policy from './i18n.policy';
import * as draft from './i18n.draft';
import * as hist from './i18n.history';

const baseTh = {
  // Menu & Heading
  label: 'เรทและสกุลเงิน',
  eyebrow: 'RATE MANAGEMENT',
  title: 'เรทและสกุลเงิน',
  subtitle: 'เรทหน้าร้านและข้อมูลอ้างอิงสำหรับสาขาที่เลือก',

  // Kiosk currency link (admin-workspace.js:137-138)
  kioskCurrencyNote: 'กำหนดสกุลเงินที่ลูกค้าเลือกได้ พร้อมเทียบเรทกสิกร',
  kioskCurrencyLink: 'จัดการสกุลเงินหน้าร้าน →',

  // Rates section (admin-html:19, :100, :68)
  ratesLiveNote: 'เรทหน้าร้านจาก เซิร์ฟเวอร์ · THB ต่อ 1 หน่วยเงินต่างประเทศ',
  ratesEyebrow: 'EXCHANGE RATES',
  ratesTitle: 'เรทหน้าร้าน',
  ratesPill: 'เรทที่ผู้ดูแลกำหนด',
  lastEditedPrefix: 'แก้ไขล่าสุด ',
  editRateAria: 'แก้ไขเรท ',

  // Rates table (admin-html:68)
  headerCurrency: 'สกุลเงิน',
  headerBuy: 'ร้านรับซื้อ',
  headerSell: 'ร้านขาย',
  headerSpread: 'ส่วนต่างเรท',
  notSet: 'ยังไม่ตั้ง',
  editRateButton: 'แก้ไขเรท',

  // Edit dialog / common
  cancel: 'ยกเลิก',
  sellBelowBuyError: 'เรทร้านขายต้องไม่น้อยกว่าเรทร้านรับซื้อ',
  noReferenceData: 'ยังไม่มีข้อมูลอ้างอิง',

  // BOT panel (rates-reference.js:7)
  botEyebrow: 'BOT REFERENCE / PARTNER COST',
  botTitle: 'เทียบเรท ธปท. และต้นทุนคู่ค้า',
  botSubtitle: 'บาทต่อ 1 หน่วยเงินต่างประเทศ · ธปท. เผยแพร่เรทถัวเฉลี่ยรายวัน · ตรวจข้อมูลทุก 60 วินาทีขณะเปิดหน้านี้',
  botLoading: 'กำลังดึงข้อมูล ธปท.…',
  botRefreshButton: 'โหลดเรท ธปท. ล่าสุด',
  rateHistoryButton: 'ประวัติการปรับเรท',
  botSourceLink: 'แหล่งข้อมูล ธปท. ↗',
  headerPair: 'คู่เงิน',
  headerShopRates: 'ร้านรับซื้อ / ขาย',
  headerBotRates: 'ธปท. ซื้อตั๋ว / โอน / ขาย',
  botFetchedPrefix: 'ดึงข้อมูล ',
  botLastFetchedPrefix: 'ดึงข้อมูลล่าสุด ',
  botDatePrefix: 'ธปท. วันที่ ',
  botSight: 'ซื้อตั๋วเงิน',
  botTransfer: 'ซื้อเงินโอน',
  botSelling: 'ขาย',

  // History dialog (rates-reference.js:8)
  historyTitlePrefix: 'ประวัติการปรับเรท · ',
  historyTitle: 'ประวัติการปรับเรท',
  headerTimeActor: 'เวลา / ผู้แก้',
  headerPairShopRate: 'คู่เงิน / เรทร้าน',
  headerVersion: 'ฉบับที่',
  headerPublishedBy: 'ผู้เผยแพร่',
  headerPublishedAt: 'เวลาที่เผยแพร่',

  // Draft section headers
  currentPublicationTitle: 'เรทที่เผยแพร่อยู่ปัจจุบัน',
  noPublication: 'ยังไม่มีการเผยแพร่เรท',

  // #4912 input bounds & tier errors
  adjustPctInvalid: 'เปอร์เซ็นต์ปรับเรทต้องเป็นตัวเลขทศนิยมไม่เกิน 4 ตำแหน่ง',
  sharpMoveInvalid: 'เกณฑ์แจ้งเตือนต้องเป็นตัวเลขมากกว่า 0 (ทศนิยมไม่เกิน 4 ตำแหน่ง)',
  tierUpperInvalidSuffix: ' ต้องระบุเพดานมูลค่ามากกว่า 0 และทศนิยมไม่เกิน 2 ตำแหน่ง',
  tierMarginInvalidSuffix: ' ต้องระบุส่วนต่างรับซื้อและขายเป็นตัวเลขทศนิยมไม่เกิน 4 ตำแหน่ง',
};

const baseEn = {
  // Menu & Heading
  label: 'Rates & Currencies',
  eyebrow: 'RATE MANAGEMENT',
  title: 'Rates & Currencies',
  subtitle: 'Branch rates and reference data for the selected branch',

  // Kiosk currency link
  kioskCurrencyNote: 'Configure customer selectable currencies and compare with Kasikorn rates',
  kioskCurrencyLink: 'Manage Kiosk Currencies →',

  // Rates section
  ratesLiveNote: 'Branch rates from server · THB per 1 foreign currency unit',
  ratesEyebrow: 'EXCHANGE RATES',
  ratesTitle: 'Branch Exchange Rates',
  ratesPill: 'Admin configured rates',
  lastEditedPrefix: 'Last edited ',
  editRateAria: 'Edit rate ',

  // Rates table
  headerCurrency: 'Currency',
  headerBuy: 'Buying Rate',
  headerSell: 'Selling Rate',
  headerSpread: 'Spread',
  notSet: 'Not set',
  editRateButton: 'Edit rate',

  // Edit dialog / common
  cancel: 'Cancel',
  sellBelowBuyError: 'Selling rate must not be less than buying rate',
  noReferenceData: 'No reference data',

  // BOT panel
  botEyebrow: 'BOT REFERENCE / PARTNER COST',
  botTitle: 'BOT Reference Rates & Partner Cost',
  botSubtitle: 'THB per 1 unit of foreign currency · BOT publishes daily weighted average · Checks every 60s while this page is open',
  botLoading: 'Fetching BOT data…',
  botRefreshButton: 'Fetch Latest BOT Rates',
  rateHistoryButton: 'Rate History',
  botSourceLink: 'BOT Source ↗',
  headerPair: 'Currency Pair',
  headerShopRates: 'Shop Buy / Sell',
  headerBotRates: 'BOT Sight / Transfer / Sell',
  botFetchedPrefix: 'Fetched ',
  botLastFetchedPrefix: 'Last fetched ',
  botDatePrefix: 'BOT Date ',
  botSight: 'Sight Bill',
  botTransfer: 'Telegraphic Transfer',
  botSelling: 'Selling',

  // History dialog
  historyTitlePrefix: 'Rate Change History · ',
  historyTitle: 'Rate Publication History',
  headerTimeActor: 'Time / Editor',
  headerPairShopRate: 'Pair / Shop Rates',
  headerVersion: 'Version',
  headerPublishedBy: 'Published by',
  headerPublishedAt: 'Published at',

  // Draft section headers
  currentPublicationTitle: 'Current Published Rates',
  noPublication: 'No published rates yet',

  // #4912 input bounds & tier errors
  adjustPctInvalid: 'Adjustment percentage must be a number with at most 4 decimal places',
  sharpMoveInvalid: 'Sharp-move threshold must be a positive number with at most 4 decimal places',
  tierUpperInvalidSuffix: ' upper limit must be above 0 with at most 2 decimal places',
  tierMarginInvalidSuffix: ' requires valid buy and sell margins with at most 4 decimal places',
};

export const th = {
  ...baseTh,
  ...policy.th,
  ...draft.th,
  ...hist.th,
};

export const en = {
  ...baseEn,
  ...policy.en,
  ...draft.en,
  ...hist.en,
};
