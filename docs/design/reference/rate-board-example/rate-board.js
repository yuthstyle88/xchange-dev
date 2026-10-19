// Branch rate board (boarding-pass theme). Polls /api/rate-board every 10 s, shows every
// currency at once on a 1080×1920 canvas scaled to the screen (no paging, no scrolling),
// flags instead of currency letters, and split-flap digits that flip when a rate changes.
// Rates are hidden after 30 s without a successful update.
// Attract motion for a screen facing passers-by: a spotlight walks down the currencies, the whole
// board does a split-flap wave every 70 s, and a welcome ticker runs above the stub. ?attract=0 turns it off.
(() => {
  'use strict';
  const $ = id => document.getElementById(id);
  const branch = new URLSearchParams(location.search).get('branch');
  const rowsEl = $('rows'), cells = new Map(); // "USD|1|buy" -> {el, text, value}
  let data = null, lastSuccess = 0, failed = false, codes = '';
  const calm = new URLSearchParams(location.search).get('attract') === '0';
  if (calm) document.body.classList.add('calm');
  const spot = document.createElement('div'); spot.id = 'spot'; spot.setAttribute('aria-hidden', 'true');
  let lit = -1, shownCount = null, minute = '';

  const decimals = v => v >= 10 ? 2 : v >= 1 ? 3 : 4;
  const format = v => v == null || !(v > 0) ? '—' : v.toLocaleString('en-US', {minimumFractionDigits: decimals(v), maximumFractionDigits: decimals(v)});
  function fit() { document.documentElement.style.setProperty('--scale', Math.min(innerWidth / 1080, innerHeight / 1920)); size(); place(); }
  function size() {
    const n = data?.rates?.length || 1;
    rowsEl.style.setProperty('--n', n);
    const h = rowsEl.clientHeight / n || 40;
    rowsEl.style.setProperty('--fs', Math.max(14, Math.min(34, h * 0.47)) + 'px');
  }

  // Characters as separate spans so only the digits that changed flip.
  function setText(cell, text, direction) {
    const old = cell.text || '';
    cell.el.querySelectorAll('.ch').forEach(c => c.remove());
    const frag = document.createDocumentFragment();
    [...text].forEach((ch, i) => {
      const s = document.createElement('span');
      s.className = 'ch' + (/[.,]/.test(ch) ? ' sep' : '');
      s.textContent = ch;
      if (old && old[old.length - text.length + i] !== ch) { s.classList.add('flip'); s.style.setProperty('--d', (i * 45) + 'ms'); }
      frag.append(s);
    });
    cell.el.append(frag);
    cell.text = text;
    if (direction) {
      cell.el.classList.remove('up', 'down'); void cell.el.offsetWidth; // restart the glow
      cell.el.classList.add(direction > 0 ? 'up' : 'down');
      cell.el.querySelector('em')?.remove();
      const arrow = document.createElement('em'); arrow.textContent = direction > 0 ? '▲' : '▼'; cell.el.prepend(arrow);
    }
  }

  const tiersOf = r => r.tiers || [1, 2, 3, 4].map(tier => ({tier, buy: r.buy, sell: r.sell}));

  function build() {
    rowsEl.replaceChildren(); cells.clear();
    data.rates.forEach(r => {
      const row = document.createElement('div');
      row.className = 'row' + (r.tiers ? '' : ' single');
      const cur = document.createElement('div'); cur.className = 'cur';
      const flag = document.createElement('img');
      flag.alt = ''; flag.src = '/flags/' + r.code.slice(0, 2).toLowerCase() + '.svg';
      flag.onerror = () => { const box = document.createElement('span'); box.className = 'code-only'; box.textContent = r.code; flag.replaceWith(box); };
      const code = document.createElement('b'); code.textContent = r.code; code.title = r.name || r.code;
      cur.append(flag, code); row.append(cur);
      for (const t of tiersOf(r)) {
        const group = document.createElement('div'); group.className = 'tier-cell';
        for (const side of ['buy', 'sell']) {
          const el = document.createElement('div'); el.className = 'price ' + side;
          group.append(el);
          const cell = {el, text: '', value: null};
          cells.set(`${r.code}|${t.tier}|${side}`, cell);
          setText(cell, format(t[side]));
          cell.value = t[side];
        }
        row.append(group);
      }
      rowsEl.append(row);
    });
    rowsEl.append(spot); lit = -1;
    size();
  }

  // Attract motion. Nothing here changes a price or hides one; it only adds movement around them.
  const idle = () => calm || document.hidden || document.body.classList.contains('offline');
  const rows = () => rowsEl.querySelectorAll('.row');
  function place() {
    const row = rows()[lit];
    if (!row) return;
    spot.style.transform = `translateY(${row.offsetTop}px)`;
    spot.style.height = row.offsetHeight + 'px';
  }
  function walk() {
    const all = rows();
    all[lit]?.classList.remove('lit');
    if (idle() || !all.length) { spot.classList.remove('on'); return; }
    lit = (lit + 1) % all.length;
    all[lit].classList.add('lit');
    place();
    spot.classList.add('on');
    spot.classList.remove('glint'); void spot.offsetWidth; spot.classList.add('glint');
  }
  // Every digit flaps once, row by row from the top, like a departure board refreshing.
  function wave() {
    if (idle()) return;
    const all = [...rows()];
    all.forEach(row => row.querySelectorAll('.ch').forEach(c => c.classList.remove('flip')));
    void rowsEl.offsetWidth;
    all.forEach((row, r) => row.querySelectorAll('.price').forEach(price => price.querySelectorAll('.ch').forEach((c, i) => {
      c.style.setProperty('--d', (r * 55 + i * 30) + 'ms'); c.classList.add('flip');
    })));
  }
  function ticker(count) {
    if (count === shownCount) return;
    shownCount = count;
    const parts = ['ยินดีต้อนรับ', 'WELCOME', '欢迎光临', 'ようこそ', '환영합니다', 'ДОБРО ПОЖАЛОВАТЬ',
      'แลกเงินได้ที่เคาน์เตอร์นี้ · EXCHANGE HERE',
      ...(count ? [`${count} สกุลเงิน · ${count} CURRENCIES`] : []),
      'ยิ่งแลกมาก เรทยิ่งดี · BETTER RATES FOR LARGER AMOUNTS'];
    const line = () => {
      const span = document.createElement('span');
      parts.forEach((text, i) => { if (i) { const sep = document.createElement('i'); sep.textContent = '✦'; span.append(sep); } span.append(text); });
      return span;
    };
    $('ticker').replaceChildren(line(), line()); // two copies so the loop at -50% is seamless
  }

  function update() {
    for (const r of data.rates) for (const t of tiersOf(r)) for (const side of ['buy', 'sell']) {
      const cell = cells.get(`${r.code}|${t.tier}|${side}`);
      if (!cell) continue;
      const text = format(t[side]);
      if (text !== cell.text) setText(cell, text, cell.value > 0 && t[side] > 0 ? Math.sign(t[side] - cell.value) : 0);
      cell.value = t[side];
    }
  }

  function status() {
    const expired = !lastSuccess || performance.now() - lastSuccess > 30000;
    document.body.classList.toggle('offline', expired);
    document.body.classList.toggle('stale', failed && !expired);
    $('status').textContent = expired ? 'งดแสดงเรท · เชื่อมต่อไม่ได้' : failed ? 'ข้อมูลล่าช้า · โปรดยืนยัน' : 'เชื่อมต่อข้อมูลสาขาแล้ว';
    $('substatus').textContent = expired ? 'Offline / no rates displayed' : failed ? 'Delayed / confirm at counter' : 'Synced with counter';
    const empty = $('empty');
    empty.hidden = !expired && !!data?.rates?.length;
    if (expired) empty.textContent = branch ? 'กรุณาสอบถามเรทที่เคาน์เตอร์ · Please ask at the counter' : 'กรุณาระบุสาขาในลิงก์ เช่น ?branch=MAIN';
    else if (data && !data.rates.length) empty.textContent = 'สาขานี้ยังไม่มีสกุลเงินที่เปิดให้บริการ';
  }

  async function poll() {
    try {
      if (!branch || !/^[A-Za-z0-9-]{1,70}$/.test(branch)) throw Error('branch');
      const response = await fetch('/api/rate-board?branch=' + encodeURIComponent(branch), {cache: 'no-store', signal: AbortSignal.timeout(7000)});
      if (!response.ok) throw Error('feed');
      const next = await response.json();
      if (next.branchId !== branch || !Array.isArray(next.rates)) throw Error('feed');
      const nextCodes = next.rates.map(r => r.code + (r.tiers ? '*' : '')).join(',');
      const changed = data?.snapshotId !== next.snapshotId;
      data = next; lastSuccess = performance.now(); failed = false;
      if (nextCodes !== codes) { codes = nextCodes; build(); } else if (changed) update();
      $('branch').textContent = next.branchId; $('name').textContent = next.branchName;
      $('count').textContent = next.rates.length; ticker(next.rates.length);
      $('checked').textContent = new Date(next.checkedAt).toLocaleTimeString('th-TH', {timeZone: 'Asia/Bangkok'});
      $('version').textContent = next.snapshotId;
    } catch { failed = true; }
    finally { status(); size(); setTimeout(poll, 10000); }
  }

  setInterval(() => {
    const now = new Date();
    const hm = now.toLocaleTimeString('en-GB', {timeZone: 'Asia/Bangkok', hour: '2-digit', minute: '2-digit'});
    if (hm !== minute) { // rebuild only when the minute changes so the colon keeps blinking smoothly
      minute = hm;
      const colon = document.createElement('i'); colon.className = 'colon'; colon.textContent = ':';
      const [h, m] = hm.split(':');
      $('clock').replaceChildren(h, colon, m);
    }
    $('date').textContent = now.toLocaleDateString('th-TH', {timeZone: 'Asia/Bangkok', weekday: 'long', day: 'numeric', month: 'long', year: 'numeric'});
    if (lastSuccess && performance.now() - lastSuccess > 30000) status();
  }, 1000);
  addEventListener('resize', fit); fit(); ticker(0); poll();
  setInterval(walk, 1900);
  setInterval(wave, 70000);
})();
