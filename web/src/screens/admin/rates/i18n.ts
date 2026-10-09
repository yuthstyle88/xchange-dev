export const th = {
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

  // Edit dialog (admin-html:134; rates-reference.js:5, :10)
  editTitlePrefix: 'ตั้งเรทจริง ',
  editNotice: 'ใช้กับรายการใหม่ของหน้าทำรายการ การแก้ไขจะบันทึกผู้แก้และเวลาใน เซิร์ฟเวอร์',
  cancel: 'ยกเลิก',
  saveToServer: 'บันทึกเรทลง เซิร์ฟเวอร์',
  sellBelowBuyError: 'เรทร้านขายต้องไม่น้อยกว่าเรทร้านรับซื้อ',
  noReferenceData: 'ยังไม่มีข้อมูลอ้างอิง',
  jpyConversionNote: ' · แปลงจากต่อ 100 เยนเป็นต่อ 1 เยนแล้ว',
  botSaveNotice: 'ระบบจะดึงและบันทึกเรท ธปท. ใหม่อีกครั้งตอนกดบันทึก หากดึงไม่ได้จะยังไม่ปรับเรทร้าน',
  partnerNameLabel: 'ชื่อคู่ค้า',
  partnerNamePlaceholder: 'ไม่ระบุได้ หากยังไม่มีต้นทุนคู่ค้า',
  partnerPairError: 'กรอกชื่อคู่ค้าและเรทต้นทุนให้ครบคู่',
  partnerCostPrefix: 'เรทที่ร้านรับซื้อจากคู่ค้า (THB ต่อ 1 ',

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
  headerPartnerCost: 'ต้นทุนจากคู่ค้า',
  headerPartnerSpread: 'ส่วนต่างขาย − ต้นทุน',
  partnerNotSpecified: 'ยังไม่ระบุ',
  botFetchedPrefix: 'ดึงข้อมูล ',
  botLastFetchedPrefix: 'ดึงข้อมูลล่าสุด ',
  botDatePrefix: 'ธปท. วันที่ ',
  botSight: 'ซื้อตั๋วเงิน',
  botTransfer: 'ซื้อเงินโอน',
  botSelling: 'ขาย',

  // History dialog (rates-reference.js:8)
  historyTitlePrefix: 'ประวัติการปรับเรท · ',
  historyNote: 'ค่า ธปท. ที่เซิร์ฟเวอร์ดึงขณะบันทึกแต่ละครั้ง · รายการเก่าอาจยังไม่มีค่าอ้างอิง',
  headerTimeActor: 'เวลา / ผู้แก้',
  headerPairShopRate: 'คู่เงิน / เรทร้าน',
  headerPartnerCostHistory: 'คู่ค้า / ต้นทุน',
  headerBotReferenceCopy: 'สำเนาเรท ธปท.',
};

export const en = {
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

  // Edit dialog
  editTitlePrefix: 'Set Live Rate ',
  editNotice: 'Applies to new transactions. Edits will record editor and timestamp on server.',
  cancel: 'Cancel',
  saveToServer: 'Save Rates to Server',
  sellBelowBuyError: 'Selling rate must not be less than buying rate',
  noReferenceData: 'No reference data',
  jpyConversionNote: ' · Converted from per 100 JPY to per 1 JPY',
  botSaveNotice: 'System will re-fetch and save BOT rates upon submitting. If failed, rates will not be updated.',
  partnerNameLabel: 'Partner Name',
  partnerNamePlaceholder: 'Optional if partner cost is not set',
  partnerPairError: 'Please provide both partner name and cost rate as a pair',
  partnerCostPrefix: 'Partner cost rate (THB per 1 ',

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
  headerPartnerCost: 'Partner Cost',
  headerPartnerSpread: 'Sell − Cost Spread',
  partnerNotSpecified: 'Not specified',
  botFetchedPrefix: 'Fetched ',
  botLastFetchedPrefix: 'Last fetched ',
  botDatePrefix: 'BOT Date ',
  botSight: 'Sight Bill',
  botTransfer: 'Telegraphic Transfer',
  botSelling: 'Selling',

  // History dialog
  historyTitlePrefix: 'Rate Change History · ',
  historyNote: 'BOT reference captured by server during each save · older records may lack reference data',
  headerTimeActor: 'Time / Editor',
  headerPairShopRate: 'Pair / Shop Rates',
  headerPartnerCostHistory: 'Partner / Cost',
  headerBotReferenceCopy: 'BOT Reference Copy',
};
