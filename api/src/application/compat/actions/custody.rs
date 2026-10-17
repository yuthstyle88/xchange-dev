//! Custody action family (`ops*`): retired by CA-10 (#626, design
//! `cash-shift-transfer.md` §6).
//!
//! All thirteen `ops*` action names (`opsEnable`, `opsSetEntity`, `opsClockIn`,
//! `opsClockOut`, `opsCashTarget`, `opsPropose`, `opsSign`, `opsExecute`,
//! `opsConfirm`, `opsCancel`, `opsForceReset`, `opsServiceOpen`, `opsCutOff`)
//! are retired from the compat RPC dispatch and answered as names v2 does not
//! know (`คำขอไม่ถูกต้อง` on `/api/rpc`, `สิทธิ์ไม่เพียงพอสำหรับใช้งานส่วนพนักงาน` on
//! `/api/staff/rpc`).
