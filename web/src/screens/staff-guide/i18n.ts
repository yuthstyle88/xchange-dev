export const th = {
  title: 'คู่มือพนักงาน XC Desk v2',
  back: 'กลับแอดมิน',
  print: 'พิมพ์คู่มือ',
  section1Heading: '1. เข้าสู่ระบบและตรวจสาขา',
  section1Body:
    'ใช้บัญชีของตนเอง ตรวจชื่อพนักงานและสาขาก่อนทำงาน เจ้าของจัดการบัญชีและบทบาท Front, Backend, Manager, Rates, Monitor และ Finance ตามสิทธิ์ที่กำหนดไว้',
  section2Heading: '2. เปิดกะและตรวจเรท',
  section2Body:
    'นับเงินจริงทุกสกุล แล้วบันทึกยอดเปิดกะตามที่นับได้ ตรวจเรทร้านรับซื้อและขายก่อนเปิดบริการ หน้าสกุลเงินแสดงเรทธนบัตรกสิกรไว้เทียบเฉพาะสกุลที่กสิกรมีเรท การเปิดสกุลเงินยังต้องกำหนดเรทร้านเอง ข้อมูล KB เป็นข้อมูลที่บันทึกไว้ ไม่ใช่ข้อมูลสด ให้ตรวจวันเวลาประกาศเสมอ',
  section3Heading: '3. ลูกค้าเลือกแลกเงิน',
  section3Body:
    'เลือกทิศทาง สกุลเงิน และจำนวนจ่าย ระบบแสดงเฉพาะเรทของทิศทางนั้นและคำนวณยอดรับอัตโนมัติ ลูกค้าตรวจยอด รอ 3 วินาที แล้วยืนยันรับเลขคิว ยอดนี้เป็นยอดประมาณการก่อนค่าบริการ',
  section4Heading: '4. รับคิวและตรวจพาสปอร์ต',
  section4Body:
    'เปิดหน้าคิวและรับเรื่องด้วยบัญชีของตนเอง เปิดหน้าทำรายการจากคิวเพื่อเติมข้อมูล ตรวจพาสปอร์ตและยืนยันด้วยตนเอง สถานะลูกค้าเปลี่ยนจาก Checking ID เป็น Pending หลังยืนยันตรวจเอกสาร ตรวจเรท จำนวนเงิน ค่าบริการ และยอดสุดท้ายกับลูกค้าอีกครั้งก่อนบันทึก',
  section5Heading: '5. บันทึกและพิมพ์ใบเสร็จ',
  section5Body1:
    'นับเงินก่อนส่งมอบ เมื่อบันทึกรายการสำเร็จจะแสดง Success พร้อมเลขรายการ พิมพ์ใบเสร็จกระดาษ 80 มม. ขนาด 100% ปิดหัวและท้ายกระดาษของเบราว์เซอร์ ใบเสร็จมีพื้นที่ท้ายเพิ่ม 20 มม. ตรวจยอด เงิน และใบเสร็จก่อนส่งลูกค้า',
  section5Body2:
    'ใบเสร็จและพาสปอร์ตผูกกับรหัสรายการ เปิดเอกสารจากรายละเอียดรายการ การทำงานกับเครื่องพิมพ์หรืออุปกรณ์จริงต้องตั้งค่าและทดสอบการเชื่อมต่อก่อน',
  section6Heading: '6. ส่งกะและปิดกะ',
  section6Body:
    'นับเงิน เทียบยอดระบบ แยกส่วนต่างเงินสดออกจากกำไรและส่วนต่างตีราคา บันทึกเหตุผลของผลต่างและหลักฐานส่งมอบ ปิดกะและลงเวลาออกหลังส่งมอบเสร็จจริง ตัวเลขกำไรประมาณการไม่ใช่คำสั่งถอนเงินอัตโนมัติ',
} as const;

export const en: Record<keyof typeof th, string> = {
  title: 'XC Desk v2 Staff Guide',
  back: 'Back to Admin',
  print: 'Print Guide',
  section1Heading: '1. Sign in and verify branch',
  section1Body:
    'Use your own account. Check employee name and branch before working. The owner manages accounts and roles Front, Backend, Manager, Rates, Monitor, and Finance according to designated permissions.',
  section2Heading: '2. Open shift and check rates',
  section2Body:
    'Count physical cash in all currencies, then record opening balance as counted. Check buying and selling rates before opening service. The currency screen displays Kasikorn banknote rates for comparison only for currencies Kasikorn provides. Enabling currencies still requires setting store rates. KB data is recorded data, not live data; always verify announcement date and time.',
  section3Heading: '3. Customer selects currency exchange',
  section3Body:
    'Select direction, currency, and payment amount. The system shows only the rate for that direction and calculates receiving amount automatically. Customer checks amount, waits 3 seconds, then confirms to receive queue number. This amount is an estimate before service fees.',
  section4Heading: '4. Call queue and check passport',
  section4Body:
    'Open queue screen and accept ticket with your own account. Open transaction screen from queue to fill in details. Check passport and confirm in person. Customer status changes from Checking ID to Pending after confirming document check. Verify rate, amount, fee, and final amount with customer again before saving.',
  section5Heading: '5. Save and print receipt',
  section5Body1:
    'Count cash before handover. When transaction is saved successfully, Success is displayed with transaction number. Print receipt on 80 mm paper at 100% size, disabling browser headers and footers. Receipt has 20 mm extra bottom margin. Verify amount, cash, and receipt before handing to customer.',
  section5Body2:
    'Receipt and passport are linked to transaction ID. Open documents from transaction details. Working with actual printers or devices requires configuration and testing connection beforehand.',
  section6Heading: '6. Hand over shift and close shift',
  section6Body:
    'Count cash, reconcile with system balance, separate cash discrepancy from profit and revaluation difference. Record reason for discrepancy and handover evidence. Close shift and clock out after handover is completed in reality. Estimated profit figure is not an automatic withdrawal order.',
};

