(function () {
  'use strict';

  const BACK_LABEL = '← กลับไปแก้ไขรายการ';
  const UNKNOWN_NOTE = 'ยังไม่ทราบผลการบันทึก กด "ลองบันทึกซ้ำ" ได้ ระบบไม่บันทึกรายการเดียวกันซ้ำ';
  const CUSTOMER_UNREACHED = 'สร้างรหัสลูกค้าไม่ได้ เชื่อมต่อเซิร์ฟเวอร์ไม่ได้ กรุณาลองใหม่';
  const STAFF_TITLE = 'รหัสจากบัญชีที่เข้าสู่ระบบ';

  function rpcAction(url, init) {
    if (!init || typeof init !== 'object') {
      return null;
    }
    const method = (init.method || 'GET').toUpperCase();
    if (method !== 'POST') {
      return null;
    }
    let pathname = '';
    try {
      pathname = new URL(url, 'http://dummy.local').pathname;
    } catch {
      return null;
    }
    if (pathname !== '/api/rpc' && pathname !== '/api/staff/rpc') {
      return null;
    }
    if (typeof init.body !== 'string') {
      return null;
    }
    let parsed;
    try {
      parsed = JSON.parse(init.body);
    } catch {
      return null;
    }
    if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) {
      return null;
    }
    if (typeof parsed.action !== 'string' || !parsed.action) {
      return null;
    }
    return parsed.action;
  }

  function readAnswer(status, text) {
    let isJson = false;
    let error = null;
    if (typeof text === 'string' && text.trim().length > 0) {
      try {
        const parsed = JSON.parse(text);
        if (parsed !== null && typeof parsed === 'object' && !Array.isArray(parsed)) {
          isJson = true;
          if (typeof parsed.error === 'string') {
            error = parsed.error;
          }
        }
      } catch {
        // not valid json
      }
    }
    return { status, json: isJson, error };
  }

  function saveOutcome(observed) {
    if (observed === null) {
      return 'refused';
    }
    if (
      observed &&
      typeof observed.status === 'number' &&
      observed.status >= 400 &&
      observed.status <= 499 &&
      observed.json &&
      typeof observed.error === 'string' &&
      observed.error.length > 0
    ) {
      return 'refused';
    }
    return 'unknown';
  }

  function customerErrorText(online, observed) {
    if (!online) {
      return null;
    }
    if (observed && typeof observed.error === 'string' && observed.error.length > 0) {
      return observed.error;
    }
    return CUSTOMER_UNREACHED;
  }

  function fillsStaff(pathname, user) {
    if (pathname !== '/staff/desk') {
      return false;
    }
    if (!user || typeof user !== 'object') {
      return false;
    }
    if (typeof user.username !== 'string' || user.username.length === 0) {
      return false;
    }
    return true;
  }

  function backToEdit(state) {
    const out = { ...state };
    const drop = [
      'booking',
      'bookedAt',
      'cloudPayload',
      'cloudResult',
      'cloudError',
      'submissionId',
      'paidAt',
      'receipt',
      'documentRef',
    ];
    for (const key of drop) {
      delete out[key];
    }
    out.ref = '';
    return out;
  }

  function buttons(view) {
    const recordMode = (view && view.recordMode) || 'LIVE';
    const cloudSaving = Boolean(view && view.cloudSaving);
    const saved = Boolean(view && view.saved);
    const outcome = (view && view.outcome) || null;

    if (recordMode === 'DEMO') {
      return { back: false, newEnabled: true, printEnabled: true, unknownNote: false };
    }
    if (cloudSaving) {
      return { back: false, newEnabled: false, printEnabled: false, unknownNote: false };
    }
    if (saved) {
      return { back: false, newEnabled: true, printEnabled: true, unknownNote: false };
    }
    if (outcome === 'refused') {
      return { back: true, newEnabled: true, printEnabled: false, unknownNote: false };
    }
    if (outcome === 'unknown') {
      return { back: false, newEnabled: false, printEnabled: false, unknownNote: true };
    }
    return { back: false, newEnabled: false, printEnabled: false, unknownNote: false };
  }

  function install(doc) {
    const win = (doc && doc.defaultView) || (typeof window !== 'undefined' ? window : null);
    if (!win) return;

    const lastActionAnswer = {};

    if (typeof win.fetch === 'function') {
      const origFetch = win.fetch;
      win.fetch = async function (input, init) {
        const action = rpcAction(input, init);
        if (!action) {
          return origFetch.apply(this, arguments);
        }
        try {
          const res = await origFetch.apply(this, arguments);
          try {
            const cloned = res.clone();
            const text = await cloned.text();
            lastActionAnswer[action] = readAnswer(res.status, text);
          } catch {
            lastActionAnswer[action] = { status: res.status, json: false, error: null };
          }
          return res;
        } catch (err) {
          lastActionAnswer[action] = { failed: true };
          throw err;
        }
      };
    }

    function updateCustomerError() {
      const errEl = doc.getElementById('customerError');
      if (errEl && errEl.textContent && errEl.textContent.trim().length > 0) {
        const isOnline =
          typeof cloudOnline !== 'undefined'
            ? Boolean(cloudOnline)
            : typeof win.cloudOnline !== 'undefined'
              ? Boolean(win.cloudOnline)
              : typeof recordMode !== 'undefined'
                ? recordMode === 'LIVE'
                : typeof win.recordMode !== 'undefined'
                  ? win.recordMode === 'LIVE'
                  : false;
        let observed = null;
        if (
          lastActionAnswer['opsBeginTransaction'] &&
          (lastActionAnswer['opsBeginTransaction'].status >= 400 ||
            lastActionAnswer['opsBeginTransaction'].failed)
        ) {
          observed = lastActionAnswer['opsBeginTransaction'];
        } else if (lastActionAnswer['reserveCustomer']) {
          observed = lastActionAnswer['reserveCustomer'];
        }
        const text = customerErrorText(isOnline, observed);
        if (text !== null && errEl.textContent !== text) {
          errEl.textContent = text;
        }
      }
    }

    function hookPassportForm() {
      const panel = doc.getElementById('panel') || doc.body;
      if (!panel) return;
      const form = panel.querySelector('form');
      if (!form) return;
      if (form.onsubmit && form.onsubmit === form.__xc_wrapped_onsubmit) return;

      const origOnSubmit = form.onsubmit;
      const wrapped = async function () {
        lastActionAnswer['opsBeginTransaction'] = null;
        lastActionAnswer['reserveCustomer'] = null;
        try {
          if (typeof origOnSubmit === 'function') {
            return await origOnSubmit.apply(this, arguments);
          }
        } finally {
          updateCustomerError();
          if (typeof queueMicrotask === 'function') {
            queueMicrotask(updateCustomerError);
          }
        }
      };
      form.__xc_wrapped_onsubmit = wrapped;
      form.onsubmit = wrapped;
    }

    const origRenderPassport =
      win.renderPassport || (typeof renderPassport === 'function' ? renderPassport : null);
    if (typeof origRenderPassport === 'function') {
      const wrappedRenderPassport = function () {
        const res = origRenderPassport.apply(this, arguments);
        hookPassportForm();
        return res;
      };
      win.renderPassport = wrappedRenderPassport;
      try {
        renderPassport = wrappedRenderPassport;
      } catch {}
    }

    let staffUsername = null;
    let mutating = false;

    if (win.location && fillsStaff(win.location.pathname, { username: 'x' })) {
      win
        .fetch('/api/auth', {
          method: 'POST',
          headers: {
            'Content-Type': 'application/json',
            'X-Xchange-Request': '1',
          },
          body: JSON.stringify({ action: 'status' }),
        })
        .then((res) => (res && res.ok ? res.json() : null))
        .then((data) => {
          if (data && fillsStaff(win.location.pathname, data.user)) {
            staffUsername = data.user.username;
            applyDomUpdates();
          }
        })
        .catch(() => {});
    }

    function applyStaff() {
      if (!staffUsername) return;
      const input = doc.getElementById('staff');
      if (input) {
        if (input.value !== staffUsername) {
          input.value = staffUsername;
        }
        if (!input.readOnly) {
          input.readOnly = true;
        }
        if (input.title !== STAFF_TITLE) {
          input.title = STAFF_TITLE;
        }
      }
      const sObj = typeof s !== 'undefined' ? s : typeof win.s !== 'undefined' ? win.s : null;
      if (sObj && (!sObj.cloudResult || !sObj.cloudResult.saved)) {
        sObj.staff = staffUsername;
      }
    }

    let currentSaveOutcome = null;

    const origGasCall = win.gasCall || (typeof gasCall === 'function' ? gasCall : null);
    if (typeof origGasCall === 'function') {
      const wrappedGasCall = async function (name, payload) {
        if (name === 'saveTransaction') {
          lastActionAnswer['saveTransaction'] = null;
          currentSaveOutcome = null;
          try {
            const ret = await origGasCall.apply(this, arguments);
            if (ret && ret.saved) {
              currentSaveOutcome = null;
            } else {
              currentSaveOutcome = saveOutcome(lastActionAnswer['saveTransaction']);
            }
            return ret;
          } catch (err) {
            currentSaveOutcome = saveOutcome(lastActionAnswer['saveTransaction']);
            throw err;
          }
        }
        return origGasCall.apply(this, arguments);
      };
      win.gasCall = wrappedGasCall;
      try {
        gasCall = wrappedGasCall;
      } catch {}
    }

    const origAccountingResult =
      win.accountingResult ||
      (typeof accountingResult === 'function' ? accountingResult : null);
    if (typeof origAccountingResult === 'function') {
      const wrappedAccountingResult = function () {
        const res = origAccountingResult.apply(this, arguments);
        applyDomUpdates();
        return res;
      };
      win.accountingResult = wrappedAccountingResult;
      try {
        accountingResult = wrappedAccountingResult;
      } catch {}
    }

    function wrapNew(newBtn) {
      if (!newBtn) return;
      if (newBtn.onclick && newBtn.onclick === newBtn.__xc_wrapped_fn) return;
      const origClick = newBtn.onclick;
      const wrapped = function () {
        const stepBefore =
          typeof step !== 'undefined' ? step : typeof win.step !== 'undefined' ? win.step : null;
        const sObjBefore = typeof s !== 'undefined' ? s : typeof win.s !== 'undefined' ? win.s : null;
        const refBefore =
          typeof ref !== 'undefined'
            ? ref
            : typeof win.ref !== 'undefined'
              ? win.ref
              : sObjBefore
                ? sObjBefore.ref
                : null;
        const savedBefore = Boolean(sObjBefore && sObjBefore.cloudResult && sObjBefore.cloudResult.saved);
        let ret;
        if (typeof origClick === 'function') {
          ret = origClick.apply(this, arguments);
        }
        const stepAfter =
          typeof step !== 'undefined' ? step : typeof win.step !== 'undefined' ? win.step : null;
        const sObjAfter = typeof s !== 'undefined' ? s : typeof win.s !== 'undefined' ? win.s : null;
        const refAfter =
          typeof ref !== 'undefined'
            ? ref
            : typeof win.ref !== 'undefined'
              ? win.ref
              : sObjAfter
                ? sObjAfter.ref
                : null;
        if (
          !origClick ||
          savedBefore ||
          stepAfter !== stepBefore ||
          refAfter !== refBefore ||
          (sObjAfter && sObjAfter !== sObjBefore)
        ) {
          currentSaveOutcome = null;
          try {
            if (typeof staffDraftId !== 'undefined') {
              staffDraftId = '';
            }
          } catch {}
          if (typeof win.staffDraftId !== 'undefined') {
            try {
              win.staffDraftId = '';
            } catch {}
          }
        }
        return ret;
      };
      newBtn.__xc_wrapped_fn = wrapped;
      newBtn.onclick = wrapped;
    }

    function applyButtons() {
      const recMode =
        typeof recordMode !== 'undefined'
          ? recordMode
          : typeof win.recordMode !== 'undefined'
            ? win.recordMode
            : 'LIVE';
      const isSaving =
        typeof cloudSaving !== 'undefined'
          ? Boolean(cloudSaving)
          : typeof win.cloudSaving !== 'undefined'
            ? Boolean(win.cloudSaving)
            : false;
      const sObj = typeof s !== 'undefined' ? s : typeof win.s !== 'undefined' ? win.s : null;
      const isSaved = Boolean(sObj && sObj.cloudResult && sObj.cloudResult.saved);
      const b = buttons({
        recordMode: recMode,
        cloudSaving: isSaving,
        saved: isSaved,
        outcome: currentSaveOutcome,
      });

      const retry = doc.getElementById('retry');
      let backBtn = doc.getElementById('xc-back');

      if (b.back) {
        if (!backBtn) {
          backBtn = doc.createElement('button');
          backBtn.id = 'xc-back';
          backBtn.type = 'button';
          backBtn.textContent = BACK_LABEL;
          backBtn.addEventListener('click', function () {
            currentSaveOutcome = null;
            let currentS =
              typeof s !== 'undefined' ? s : typeof win.s !== 'undefined' ? win.s : null;
            if (currentS) {
              const updated = backToEdit(currentS);
              let assigned = false;
              try {
                s = updated;
                assigned = true;
              } catch {}
              if (!assigned) {
                for (const drop of [
                  'booking',
                  'bookedAt',
                  'cloudPayload',
                  'cloudResult',
                  'cloudError',
                  'submissionId',
                  'paidAt',
                  'receipt',
                  'documentRef',
                ]) {
                  delete currentS[drop];
                }
                currentS.ref = '';
              }
            }
            try {
              if (typeof ref !== 'undefined') {
                ref = '';
              }
            } catch {}
            try {
              step = 0;
            } catch {}
            try {
              if (typeof staffDraftId !== 'undefined') {
                staffDraftId = '';
              }
            } catch {}
            if (typeof win.staffDraftId !== 'undefined') {
              try {
                win.staffDraftId = '';
              } catch {}
            }
            if (typeof render === 'function') {
              render();
            } else if (typeof win.render === 'function') {
              win.render();
            }
          });
        }
        if (retry && retry.parentNode && backBtn.parentNode !== retry.parentNode) {
          retry.parentNode.insertBefore(backBtn, retry.nextSibling);
        } else if (!retry && !backBtn.parentNode) {
          const panel = doc.getElementById('panel') || doc.body;
          if (panel) panel.appendChild(backBtn);
        }
      } else if (backBtn) {
        backBtn.remove();
      }

      let noteEl = doc.getElementById('xc-save-unknown');
      if (b.unknownNote) {
        if (!noteEl) {
          noteEl = doc.createElement('p');
          noteEl.id = 'xc-save-unknown';
          noteEl.textContent = UNKNOWN_NOTE;
        }
        if (retry && retry.parentNode && noteEl.parentNode !== retry.parentNode) {
          retry.parentNode.insertBefore(noteEl, retry.nextSibling);
        } else if (!retry && !noteEl.parentNode) {
          const panel = doc.getElementById('panel') || doc.body;
          if (panel) panel.appendChild(noteEl);
        }
      } else if (noteEl) {
        noteEl.remove();
      }

      const newBtn = doc.getElementById('new');
      if (newBtn) {
        newBtn.disabled = !b.newEnabled;
        wrapNew(newBtn);
      }

      const printBtn = doc.getElementById('print');
      if (printBtn) {
        printBtn.disabled = !b.printEnabled;
      }
    }

    function applyDomUpdates() {
      if (mutating) return;
      mutating = true;
      try {
        applyStaff();
        hookPassportForm();
        updateCustomerError();
        if (doc.getElementById('retry') || doc.getElementById('new') || doc.getElementById('xc-back')) {
          applyButtons();
        }
      } finally {
        mutating = false;
      }
    }

    const panel = doc.getElementById('panel');
    if (panel && typeof win.MutationObserver === 'function') {
      const observer = new win.MutationObserver(function () {
        applyDomUpdates();
      });
      observer.observe(panel, { childList: true, subtree: true });
    } else if (typeof win.MutationObserver === 'function') {
      const bodyObserver = new win.MutationObserver(function () {
        const p = doc.getElementById('panel');
        if (p) {
          bodyObserver.disconnect();
          const observer = new win.MutationObserver(function () {
            applyDomUpdates();
          });
          observer.observe(p, { childList: true, subtree: true });
        }
        applyDomUpdates();
      });
      bodyObserver.observe(doc.body || doc.documentElement, { childList: true, subtree: true });
    }

    doc.addEventListener('click', function (e) {
      const target = e.target;
      if (target && (target.id === 'new' || (target.closest && target.closest('#new')))) {
        const sObj = typeof s !== 'undefined' ? s : typeof win.s !== 'undefined' ? win.s : null;
        if (sObj && sObj.cloudResult && sObj.cloudResult.saved) {
          currentSaveOutcome = null;
          try {
            if (typeof staffDraftId !== 'undefined') {
              staffDraftId = '';
            }
          } catch {}
          if (typeof win.staffDraftId !== 'undefined') {
            try {
              win.staffDraftId = '';
            } catch {}
          }
        }
      }
    });

    applyDomUpdates();
  }

  if (typeof module === 'object' && module.exports) {
    module.exports = {
      BACK_LABEL,
      UNKNOWN_NOTE,
      CUSTOMER_UNREACHED,
      STAFF_TITLE,
      rpcAction,
      readAnswer,
      saveOutcome,
      customerErrorText,
      fillsStaff,
      backToEdit,
      buttons,
    };
  } else if (typeof document !== 'undefined') {
    install(document);
  }
})();
