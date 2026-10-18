import { afterEach, describe, expect, it } from 'vitest';
import { ApiError } from '../api/client';
import { errorText, setAudience, setLang } from './index';
import { wordedError } from './worded-error';

const refusal = (status: number, code: string, message: string) =>
  new ApiError({ status, code, message });

afterEach(() => {
  setAudience('staff');
  setLang('th');
});

describe('RBAC refusal error wording (§9)', () => {
  describe('Thai (th)', () => {
    it('statusNotAllowed (409)', () => {
      setLang('th');
      expect(errorText('statusNotAllowed', 'raw')).toBe('เปลี่ยนสถานะบัญชีนี้ไม่ได้');
      expect(wordedError(refusal(409, 'statusNotAllowed', 'raw'))).toBe('เปลี่ยนสถานะบัญชีนี้ไม่ได้');
    });

    it('selfChange (403)', () => {
      setLang('th');
      expect(errorText('selfChange', 'raw')).toBe('แก้ไขบัญชีของตัวเองไม่ได้');
      expect(wordedError(refusal(403, 'selfChange', 'raw'))).toBe('แก้ไขบัญชีของตัวเองไม่ได้');
    });

    it('lastOwner (409)', () => {
      setLang('th');
      expect(errorText('lastOwner', 'raw')).toBe('ต้องมีเจ้าของที่ยังใช้งานอยู่อย่างน้อยหนึ่งคน');
      expect(wordedError(refusal(409, 'lastOwner', 'raw'))).toBe('ต้องมีเจ้าของที่ยังใช้งานอยู่อย่างน้อยหนึ่งคน');
    });

    it('holdsCash (409)', () => {
      setLang('th');
      expect(errorText('holdsCash', 'raw')).toBe('พนักงานคนนี้ยังถือเงินสดหรือมีการส่งมอบที่ยังไม่เสร็จ');
      expect(wordedError(refusal(409, 'holdsCash', 'raw'))).toBe('พนักงานคนนี้ยังถือเงินสดหรือมีการส่งมอบที่ยังไม่เสร็จ');
    });

    it('notBound (409)', () => {
      setLang('th');
      expect(errorText('notBound', 'raw')).toBe('บัญชีนี้ยังเข้าระบบไม่ได้ ให้เปิดใช้งานก่อน');
      expect(wordedError(refusal(409, 'notBound', 'raw'))).toBe('บัญชีนี้ยังเข้าระบบไม่ได้ ให้เปิดใช้งานก่อน');
    });

    it('userNotActive (409)', () => {
      setLang('th');
      expect(errorText('userNotActive', 'raw')).toBe('บัญชีนี้ไม่ได้ใช้งานอยู่');
      expect(wordedError(refusal(409, 'userNotActive', 'raw'))).toBe('บัญชีนี้ไม่ได้ใช้งานอยู่');
    });
  });

  describe('English (en)', () => {
    it('statusNotAllowed (409)', () => {
      setLang('en');
      expect(errorText('statusNotAllowed', 'raw')).toBe('This account status change is not allowed');
      expect(wordedError(refusal(409, 'statusNotAllowed', 'raw'))).toBe('This account status change is not allowed');
    });

    it('selfChange (403)', () => {
      setLang('en');
      expect(errorText('selfChange', 'raw')).toBe('You cannot change your own account');
      expect(wordedError(refusal(403, 'selfChange', 'raw'))).toBe('You cannot change your own account');
    });

    it('lastOwner (409)', () => {
      setLang('en');
      expect(errorText('lastOwner', 'raw')).toBe('At least one owner must stay active');
      expect(wordedError(refusal(409, 'lastOwner', 'raw'))).toBe('At least one owner must stay active');
    });

    it('holdsCash (409)', () => {
      setLang('en');
      expect(errorText('holdsCash', 'raw')).toBe('This person still holds cash or a hand-over in progress');
      expect(wordedError(refusal(409, 'holdsCash', 'raw'))).toBe('This person still holds cash or a hand-over in progress');
    });

    it('notBound (409)', () => {
      setLang('en');
      expect(errorText('notBound', 'raw')).toBe('This account cannot sign in yet; activate it first');
      expect(wordedError(refusal(409, 'notBound', 'raw'))).toBe('This account cannot sign in yet; activate it first');
    });

    it('userNotActive (409)', () => {
      setLang('en');
      expect(errorText('userNotActive', 'raw')).toBe('This account is not active');
      expect(wordedError(refusal(409, 'userNotActive', 'raw'))).toBe('This account is not active');
    });
  });

  describe('Chinese (zh)', () => {
    it('statusNotAllowed (409)', () => {
      setAudience('customer');
      setLang('zh');
      expect(errorText('statusNotAllowed', 'raw')).toBe('不允许更改此账户状态');
      expect(wordedError(refusal(409, 'statusNotAllowed', 'raw'))).toBe('不允许更改此账户状态');
    });

    it('selfChange (403)', () => {
      setAudience('customer');
      setLang('zh');
      expect(errorText('selfChange', 'raw')).toBe('不能更改您自己的账户');
      expect(wordedError(refusal(403, 'selfChange', 'raw'))).toBe('不能更改您自己的账户');
    });

    it('lastOwner (409)', () => {
      setAudience('customer');
      setLang('zh');
      expect(errorText('lastOwner', 'raw')).toBe('至少须保留一位在用的所有者');
      expect(wordedError(refusal(409, 'lastOwner', 'raw'))).toBe('至少须保留一位在用的所有者');
    });

    it('holdsCash (409)', () => {
      setAudience('customer');
      setLang('zh');
      expect(errorText('holdsCash', 'raw')).toBe('此人仍持有现金或有未完成的交接');
      expect(wordedError(refusal(409, 'holdsCash', 'raw'))).toBe('此人仍持有现金或有未完成的交接');
    });

    it('notBound (409)', () => {
      setAudience('customer');
      setLang('zh');
      expect(errorText('notBound', 'raw')).toBe('此账户尚不能登录，请先启用');
      expect(wordedError(refusal(409, 'notBound', 'raw'))).toBe('此账户尚不能登录，请先启用');
    });

    it('userNotActive (409)', () => {
      setAudience('customer');
      setLang('zh');
      expect(errorText('userNotActive', 'raw')).toBe('此账户未在使用中');
      expect(wordedError(refusal(409, 'userNotActive', 'raw'))).toBe('此账户未在使用中');
    });
  });
});

