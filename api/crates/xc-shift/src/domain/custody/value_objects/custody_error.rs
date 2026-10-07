//! Custody errors and localized error messages.

use xc_kernel::money::CurrencyCode;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CustodyError {
    #[error("บทบาทนี้ไม่มีสิทธิ์ทำรายการ")]
    RoleNotAllowed,

    #[error("ลงเวลาเข้าก่อนทำรายการ")]
    NotClockedIn,

    #[error("ยังไม่มีกะเปิด")]
    NoOpenShift,

    #[error("ต้องแนบหลักฐานการนับหรือรับ–ส่งเงิน")]
    EvidenceRequired,

    #[error("กรอกจำนวนธนบัตร/เหรียญแยกชนิดราคา")]
    DenominationsRequired,

    #[error("ชนิดราคาหรือจำนวนไม่ถูกต้อง")]
    DenominationInvalid,

    #[error("ยอดชนิดราคาไม่ตรงกับยอดนับ {0}")]
    DenominationMismatch(CurrencyCode),

    #[error("กำหนดกิจการก่อนเปิดใช้และก่อนเปิดกะ")]
    EntityLocked,

    #[error("ยืนยันกิจการและการแยกบัญชีให้ครบ")]
    EntityIncomplete,

    #[error("ปิดกะเดิมก่อนเปิดใช้ขั้นตอนใหม่")]
    ShiftStillOpen,

    #[error("ยืนยันสาขาและกิจการก่อนเปิดใช้")]
    EntityMissing,

    #[error("เจ้าของต้องเปิดใช้ขั้นตอนรับ–ส่งเงินก่อน")]
    NotEnabled,

    #[error("ต้องใช้บัญชีพนักงาน")]
    EmployeeRequired,

    #[error("ยังไม่ได้ลงเวลาเข้า")]
    NoClockInToClose,

    #[error("ส่งมอบเงินและงานที่ค้างให้เสร็จก่อนลงเวลาออก")]
    DutiesOutstanding,

    #[error("ต้องเป็น Front ผู้ถือครองเงินในกะที่เปิด")]
    NotCustodianInOpenService,

    #[error("ปิดรับรายการใหม่แล้ว")]
    OutsideServiceHours,

    #[error("รายการไม่ถูกต้อง")]
    DraftInvalid,

    #[error("กรุณากรอกจำนวนเงิน")]
    AmountMissing,

    #[error("จำนวนเงินต้องไม่ติดลบ ไม่เกิน 100,000,000 และมีทศนิยมไม่เกิน 2 ตำแหน่ง")]
    AmountInvalid,

    #[error("เปิดบริการได้หลังเตรียมกะเสร็จ ระหว่าง 11:00–22:50")]
    ServiceNotReady,

    #[error("ยืนยันอุปกรณ์และเรทที่แสดงให้ตรงกับชุดเรทล่าสุด")]
    ServiceChecksIncomplete,

    #[error("อยู่ระหว่างตรวจนับ")]
    CountingUnderWay,

    #[error("คำขอเดิมมีข้อมูลเปลี่ยน")]
    RequestChanged,

    #[error("รหัสคำขอไม่ถูกต้อง")]
    RequestIdInvalid,

    #[error("เซฟมียอดตั้งต้นแล้ว")]
    SafeAlreadyOpened,

    #[error("ต้องเป็นผู้ถือครองลิ้นชักของกะนี้")]
    NotDrawerCustodian,

    #[error("มีรายการรับ–ส่งที่ยังไม่เสร็จ กรุณาตรวจรายการเดิมก่อน")]
    RequestPending,

    #[error("หมายเหตุยาวเกินกำหนด")]
    NoteTooLong,

    #[error("ข้อมูลข้อความไม่ครบหรือยาวเกินกำหนด")]
    TextInvalid,

    #[error("ผู้ดูแลเรทต้องตั้งเรทจริงก่อนเปิดกะ")]
    NoLiveRate,

    #[error("มีกะเปิดอยู่แล้ว")]
    ShiftAlreadyOpen,

    #[error("Finance ต้องกำหนดเงินสดเป้าหมายก่อน")]
    TargetMissing,

    #[error("Backend ต้องตัดรายการและล็อกการสร้างรายการก่อนนับเงิน")]
    CutoffRequired,

    #[error("มีเอกสารธุรกรรมค้าง ต้องสะสางก่อนปิดหรือส่งกะ")]
    TransactionsPending,

    #[error("ยังไม่ได้ยืนยันยอดตั้งต้นเซฟ")]
    SafeNotOpened,

    #[error("ระบุผู้รับกะคนละคนกับผู้ส่ง")]
    HandoverReceiverInvalid,

    #[error("กำลังตรวจนับปิดกะ")]
    ClosingCountUnderWay,

    #[error("เลือกสกุลเงินและผู้รับ–ส่งคนละคน")]
    TransferPartiesInvalid,

    #[error("ระบุคำอธิบายผลต่างเงินสด โดยไม่ปรับยอดให้ตรง")]
    VarianceNeedsNote,

    #[error("เลือกลูกค้าและระบุเหตุพักตรวจ")]
    HoldInvalid,

    #[error("ระบุผลตรวจรายการที่พัก")]
    ReleaseInvalid,

    #[error("ไม่พบรายการรับ–ส่ง")]
    RequestNotFound,

    #[error("เงินอยู่ระหว่างนำส่ง ต้องยืนยันการรับหรือจัดการคืนเงินตามหลักฐานก่อน")]
    MoneyInTransit,

    #[error("รายการนี้สิ้นสุดแล้ว")]
    RequestFinished,

    #[error("ระบุเหตุผลส่งกลับ")]
    RejectReasonRequired,

    #[error("รอ Backend บันทึกส่งเงินก่อนยืนยันรับ")]
    AwaitingDispatch,

    #[error("ผู้รับกะไม่ตรงกับรายการ")]
    NotTheReceiver,

    #[error("ผู้ยืนยันการเงินไม่ตรงกับรายการ")]
    NotAFinanceRequest,

    #[error("ไม่มีสิทธิ์ลงชื่อ")]
    SignNotAllowed,

    #[error("Backend ใช้ปุ่มตรวจและบันทึกงาน")]
    BackendCommitsInstead,

    #[error("รายการนี้ส่งเงินไม่ได้")]
    NotDispatchable,

    #[error("กะเปลี่ยนแล้ว")]
    ShiftChanged,

    #[error("เงินในเซฟไม่เพียงพอ")]
    SafeInsufficient,

    #[error("Backend ต้องเป็นคนละคนกับผู้รับ–ส่ง")]
    BackendIsParty,

    #[error("รอผู้จัดการจุดรับรอง")]
    AwaitingManager,

    #[error("รอ Monitor ตรวจและลงชื่อ")]
    AwaitingMonitor,

    #[error("รอผู้รับกะลงชื่อ")]
    AwaitingReceiver,

    #[error("รอ Finance ยืนยันการรับ–ส่ง")]
    AwaitingFinance,

    #[error("ยอดเงินเปลี่ยน ต้องตรวจนับใหม่")]
    BalancesChanged,

    #[error("ต้องบันทึกส่งเงินเข้าระหว่างนำส่งก่อน")]
    DispatchRequired,

    #[error("ผู้กระทบยอดต้องเป็นคนละคนกับผู้รับ–ส่งหรือผู้อนุมัติรายการนี้")]
    ReconcilerIsParty,

    #[error("งานยังไม่เสร็จ")]
    NotCompleted,

    #[error("ระบุผลตรวจและหลักฐานอ้างอิง")]
    ReconcileNoteRequired,

    #[error("คำขอเดิมไม่ใช่ของพนักงานคนนี้")]
    SaveNotOwnRetry,

    #[error("ลูกค้านี้อยู่ระหว่างพักตรวจ ให้ Monitor ยืนยันผลก่อนทำรายการ")]
    CustomerOnHold,

    #[error("ปิดรับรายการใหม่แล้ว หรือกำลังส่งกะ/ตรวจนับเงิน")]
    SaveClosed,

    #[error("ต้องเป็น Front ผู้รับผิดชอบลิ้นชักที่ลงเวลาเข้าแล้ว")]
    SaveNotCustodian,
}
