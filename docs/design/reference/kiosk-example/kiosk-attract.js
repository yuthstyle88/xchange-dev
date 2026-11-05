// Kiosk screensaver: a 36 s animated how-to (choose → review → queue number → counter) that
// kiosk.js shows after the screen has reset for inactivity. Each loop runs in the next kiosk
// language (texts from kiosk-i18n.js), with English, or Thai when the loop is English, as the
// second line. A tap or key hides it and fires `kiosk-attract-dismiss`; kiosk.js then returns
// to the start screen. Wrapped so its names stay out of kiosk.js's global scope.
(() => {
  const DURATION = 36;
  const SCENES = [['xa-s0', 0, 4.8], ['xa-s1', 4.5, 13.3], ['xa-s2', 13, 20.3], ['xa-s3', 20, 27.8], ['xa-s4', 27.5, 32.8], ['xa-s5', 32.5, 36.3]];
  // Taps the finger cursor performs: [time, target id].
  const TAPS = [[6.6, 'xa-usd'], [8.0, 'xa-k1'], [8.6, 'xa-k0'], [9.2, 'xa-k0'], [11.8, 'xa-review'], [18.4, 'xa-confirm']];
  const HAND = '<svg viewBox="0 0 110 110" aria-hidden="true"><path d="M44 58V22a8 8 0 0116 0v30l4-1V42a8 8 0 0116 0v12a8 8 0 0116 2v20c0 16-12 28-28 28H60c-9 0-16-4-21-11L24 72a8 8 0 0112-10l8 8z" fill="#fff"/></svg>';
  const TILES = [['USD', '$', 'xa-usd'], ['EUR', '€'], ['CNY', '¥'], ['JPY', '¥'], ['KRW', '₩'], ['GBP', '£']];

  let root, stage, scale = 1, raf = 0, t0 = 0, loop = -1, order = [], lang = 'th', alt = 'en';
  const $ = id => document.getElementById(id);
  const clamp = x => Math.max(0, Math.min(1, x));
  const prog = (t, a, b) => clamp((t - a) / (b - a));
  const eo = x => 1 - Math.pow(1 - x, 3);
  const eio = x => x < .5 ? 4 * x * x * x : 1 - Math.pow(-2 * x + 2, 3) / 2;
  const fill = (s, v) => String(s || '').replace(/\{(\w+)\}/g, (_, k) => v?.[k] ?? '');
  const t = (k, v) => fill(window.KIOSK_TEXT?.[lang]?.[k] ?? window.KIOSK_TEXT?.en?.[k], v);
  const t2 = (k, v) => fill(window.KIOSK_TEXT?.[alt]?.[k] ?? window.KIOSK_TEXT?.en?.[k], v);

  function languages() {
    const langs = [...(window.KIOSK_LANGS || [{ code: 'th' }, { code: 'en' }])].sort((a, b) => (a.rank ?? 99) - (b.rank ?? 99));
    const codes = langs.map(l => l.code).filter(c => window.KIOSK_TEXT?.[c]);
    return ['th', 'en', ...codes.filter(c => c !== 'th' && c !== 'en')].filter(c => codes.includes(c));
  }

  function build() {
    root = document.createElement('div');
    root.id = 'xa'; root.hidden = true; root.setAttribute('role', 'button');
    const logo = document.querySelector('.kiosk-brand img')?.src || '';
    root.innerHTML = `<div class="xa-stage" id="xa-stage">
<div class="xa-bg"></div><div class="xa-band"></div>
<div class="xa-brand"><div class="xa-logo"><img alt="" src="${logo}"></div><div><b>XCHANGE</b><small>CURRENCY EXCHANGE</small></div></div>
<div class="xa-top"><span class="xa-lang" id="xa-lang"></span><div class="xa-steps" id="xa-steps"><i>1</i><i>2</i><i>3</i></div></div>
<section class="xa-scene" id="xa-s0">
 <div class="xa-hero xa-fit" id="xa-hero0"><div class="xa-big" data-k="welcome"></div><div class="xa-mid" data-k="title"></div><div class="xa-sub" data-k2="title"></div></div>
 <div class="xa-trio">${[1, 2, 3].map(n => `<div><i>${n}</i><div class="xa-fit"><span data-k="step${n}"></span><small data-k2="step${n}"></small></div></div>`).join('')}</div>
</section>
<section class="xa-scene" id="xa-s1">
 <div class="xa-device">
  <div class="xa-dhead"><span class="xa-fit" data-k="step1"></span><span class="xa-ex" data-k="example"></span></div>
  <div class="xa-dbody">
   <div class="xa-dirs"><div class="xa-dir sel xa-fit"><span data-k="buy"></span><small data-k="buyHint"></small></div><div class="xa-dir xa-fit"><span data-k="sell"></span><small data-k="sellHint"></small></div></div>
   <div class="xa-lbl xa-fit" data-k="currencyLabel"></div>
   <div class="xa-tiles">${TILES.map(([code, sym, id]) => `<div class="xa-tile"${id ? ` id="${id}"` : ''}><span class="xa-sym">${sym}</span><b>${code}</b></div>`).join('')}</div>
   <div class="xa-lbl xa-fit" data-k="amountLabel" data-v='{"code":"USD"}'></div>
   <div class="xa-amount"><div class="xa-field" id="xa-field"><span id="xa-amt"></span><small>USD</small></div>
    <div class="xa-keys"><div class="xa-key" id="xa-k1">1</div><div class="xa-key">2</div><div class="xa-key">3</div><div class="xa-key">4</div><div class="xa-key">5</div><div class="xa-key" id="xa-k0">0</div></div></div>
   <div class="xa-est xa-fit" id="xa-est" data-k="estimate" data-v='{"amount":"3,339.00 THB"}'></div>
   <div class="xa-btn xa-fit" id="xa-review" data-k="review"></div>
  </div>
 </div>
 <div class="xa-cap xa-fit"><div class="xa-tag" data-k="step1"></div><div class="xa-h1" data-k="subtitle"></div><div class="xa-p" data-k2="subtitle"></div></div>
</section>
<section class="xa-scene" id="xa-s2">
 <div class="xa-device">
  <div class="xa-dhead"><span class="xa-fit" data-k="step2"></span><span class="xa-ex" data-k="example"></span></div>
  <div class="xa-dbody">
   <div class="xa-h3 xa-fit" data-k="reviewTitle"></div>
   <div class="xa-row"><span class="xa-fit" data-k="youGive"></span><b>100 USD</b></div>
   <div class="xa-row"><span class="xa-fit" data-k="rate"></span><b>33.39</b></div>
   <div class="xa-row big"><span class="xa-fit" data-k="youReceive"></span><b>3,339.00 ฿</b></div>
   <div class="xa-note xa-fit" data-k="feeNote"></div>
   <div class="xa-two"><div class="xa-ghost xa-fit" data-k="back"></div><div class="xa-btn off xa-fit" id="xa-confirm"></div></div>
  </div>
 </div>
 <div class="xa-cap xa-fit"><div class="xa-tag" data-k="step2"></div><div class="xa-h1" data-k="reviewTitle"></div><div class="xa-p" data-k2="reviewTitle"></div></div>
</section>
<section class="xa-scene" id="xa-s3">
 <div class="xa-device">
  <div class="xa-dhead"><span class="xa-fit" data-k="step3"></span><span class="xa-ex" data-k="example"></span></div>
  <div class="xa-dbody">
   <div class="xa-ok" id="xa-ok">✓</div>
   <div class="xa-sent xa-fit" data-k="sentTitle"></div>
   <div class="xa-queue" id="xa-queue">K-3F9A12C0</div>
   <div class="xa-bring">
    <div id="xa-cash"><svg viewBox="0 0 150 110"><rect x="10" y="22" width="130" height="70" rx="10" fill="#1d7448"/><rect x="18" y="30" width="114" height="54" rx="6" fill="none" stroke="#bfe3cc" stroke-width="3"/><circle cx="75" cy="57" r="17" fill="#bfe3cc"/><text x="75" y="65" text-anchor="middle" font-size="24" font-weight="700" fill="#1d7448">฿</text></svg></div>
    <div id="xa-pass"><svg viewBox="0 0 150 110"><rect x="40" y="6" width="72" height="98" rx="10" fill="#7a002c"/><circle cx="76" cy="46" r="16" fill="none" stroke="#e8c27a" stroke-width="3"/><path d="M60 46h32M76 30c-7 8-7 24 0 32M76 30c7 8 7 24 0 32" stroke="#e8c27a" stroke-width="2.5" fill="none"/><rect x="56" y="76" width="40" height="5" rx="2" fill="#e8c27a"/></svg></div>
   </div>
   <div class="xa-req xa-fit" data-k="requestOnly"></div>
  </div>
 </div>
 <div class="xa-cap xa-fit"><div class="xa-tag" data-k="step3"></div><div class="xa-h1" data-k="sentTitle"></div><div class="xa-p" data-k="bring"></div></div>
</section>
<section class="xa-scene" id="xa-s4">
 <div class="xa-device">
  <div class="xa-dhead"><span class="xa-fit" data-k="status"></span><span class="xa-ex">K-3F9A12C0</span></div>
  <div class="xa-dbody">
   <div class="xa-track" id="xa-track"><div class="xa-bar" id="xa-bar"><i id="xa-barfill"></i></div>
    ${['stepId', 'stepPending', 'stepSuccess'].map((k, i) => `<div class="xa-trow"><div class="xa-dot" id="xa-d${i}">${i + 1}</div><div class="xa-fit"><span data-k="${k}"></span><small data-k2="${k}"></small></div></div>`).join('')}
   </div>
   <div class="xa-receipt" id="xa-receipt"><svg viewBox="0 0 90 90"><path d="M20 8h50v74l-8-6-8 6-8-6-9 6-8-6-9 6z" fill="#fff" stroke="#1d7448" stroke-width="4" stroke-linejoin="round"/><path d="M31 30h28M31 42h28M31 54h18" stroke="#1d7448" stroke-width="4" stroke-linecap="round"/></svg><div class="xa-fit" data-k="summaryNote"></div></div>
  </div>
 </div>
 <div class="xa-cap xa-fit"><div class="xa-tag" data-k="status"></div><div class="xa-h1" data-k="phaseProcessing"></div><div class="xa-p" data-k2="phaseProcessing"></div></div>
</section>
<section class="xa-scene" id="xa-s5">
 <div class="xa-hero xa-fit"><div class="xa-big" data-k="touchStart"></div><div class="xa-sub" data-k2="touchStart"></div></div>
 <div class="xa-touch"><div class="r" id="xa-r1"></div><div class="r" id="xa-r2"></div><div class="c">${HAND}</div></div>
 <div class="xa-langs" id="xa-langs"></div>
</section>
<div class="xa-hint" id="xa-hint">${HAND}<span class="xa-fit" id="xa-hint-text"></span></div>
<div class="xa-foot">BEYOND CURRENCIES · EVERY TRIP &amp; JOURNEY</div>
<div id="xa-ripple"></div><div id="xa-cursor"></div>
</div>`;
    document.body.append(root);
    stage = $('xa-stage');
    // A click (not pointerdown) so the tap cannot land on the kiosk underneath.
    root.addEventListener('click', dismiss);
    document.addEventListener('keydown', e => { if (!root.hidden) { e.preventDefault(); dismiss(); } }, true);
    addEventListener('resize', () => { if (!root.hidden) layout(); });
  }

  function layout() {
    const portrait = innerHeight > innerWidth;
    stage.className = 'xa-stage ' + (portrait ? 'xa-port' : 'xa-land');
    const [w, h] = portrait ? [1080, 1920] : [1920, 1080];
    scale = Math.min(innerWidth / w, innerHeight / h);
    stage.style.transform = `translate(-50%,-50%) scale(${scale})`;
    applyTexts();
  }

  // Shrinks a fixed-size box's font until its text fits (children size in em).
  function fit(box) {
    box.style.fontSize = '';
    let size = parseFloat(getComputedStyle(box).fontSize);
    const min = size * .45;
    const kids = [...box.querySelectorAll(':scope > .xa-big,:scope > .xa-mid,:scope > .xa-sub,:scope > .xa-h1,:scope > .xa-p,:scope > .xa-tag')];
    kids.forEach(k => { k.style.fontSize = ''; });
    const base = kids.map(k => parseFloat(getComputedStyle(k).fontSize));
    for (let f = 1; f > .45 && (box.scrollHeight > box.clientHeight + 1 || box.scrollWidth > box.clientWidth + 1); f -= .05) {
      if (kids.length) kids.forEach((k, i) => { k.style.fontSize = base[i] * f + 'px'; });
      else { size = Math.max(min, size * .95); box.style.fontSize = size + 'px'; }
    }
  }

  function applyTexts() {
    const meta = (window.KIOSK_LANGS || []).find(l => l.code === lang);
    root.dir = meta?.dir === 'rtl' ? 'rtl' : 'ltr';
    stage.lang = meta?.lang || lang;
    for (const el of root.querySelectorAll('[data-k]')) el.textContent = t(el.dataset.k, el.dataset.v && JSON.parse(el.dataset.v));
    for (const el of root.querySelectorAll('[data-k2]')) el.textContent = lang === alt ? '' : t2(el.dataset.k2);
    root.setAttribute('aria-label', t('touchStart'));
    $('xa-lang').textContent = meta?.name || lang;
    $('xa-hint-text').textContent = t('touchStart') + (lang === alt ? '' : ' · ' + t2('touchStart'));
    $('xa-langs').replaceChildren(...order.map(code => {
      const chip = document.createElement('span');
      chip.className = 'xa-chip' + (code === lang ? ' on' : '');
      chip.textContent = (window.KIOSK_LANGS || []).find(l => l.code === code)?.name || code;
      return chip;
    }));
    for (const box of root.querySelectorAll('.xa-fit')) fit(box);
  }

  function setLanguage(i) {
    lang = order[i % order.length] || 'en';
    alt = lang === 'en' ? 'th' : 'en';
    applyTexts();
  }

  function sceneAlpha(time, a, b) {
    // The outro fades back into the intro, so the loop has no hard cut.
    if (a === 0) return 1 - prog(time, b - .3, b);
    return Math.min(prog(time, a, a + .3), 1 - prog(time, b - .3, b));
  }
  function center(id) {
    const r = $(id).getBoundingClientRect(), s = stage.getBoundingClientRect();
    return [(r.left + r.width / 2 - s.left) / scale, (r.top + r.height / 2 - s.top) / scale];
  }

  function render(time) {
    for (const [id, a, b] of SCENES) {
      const al = sceneAlpha(time, a, b), el = $(id);
      el.style.opacity = al;
      el.style.transform = `translateY(${(1 - eo(al)) * 40}px)`;
      el.style.visibility = al > 0 ? 'visible' : 'hidden';
    }
    const step = time < 4.5 || time >= 32.5 ? 0 : time < 13 ? 1 : time < 20 ? 2 : 3;
    [...$('xa-steps').children].forEach((d, i) => { d.className = i + 1 === step ? 'on' : i + 1 < step ? 'done' : ''; });

    root.querySelectorAll('.xa-trio > div').forEach((d, i) => {
      const p = eo(prog(time, .9 + i * .45, 1.6 + i * .45));
      d.style.opacity = p; d.style.transform = `translateX(${(1 - p) * -60}px)`;
    });

    $('xa-usd').classList.toggle('sel', time >= 6.6);
    const digits = time >= 9.2 ? '100' : time >= 8.6 ? '10' : time >= 8.0 ? '1' : '';
    const typing = time >= 7.4 && time < 12;
    $('xa-field').classList.toggle('focus', typing);
    $('xa-amt').innerHTML = (digits || '<span class="ph">0.00</span>') + (typing && Math.floor(time * 2) % 2 === 0 ? '<span class="xa-caret"></span>' : '');
    $('xa-est').style.opacity = eo(prog(time, 9.6, 10.2));
    $('xa-review').style.transform = `scale(${time >= 11.8 && time < 12.1 ? .96 : 1})`;

    const ready = time >= 17.6, confirm = $('xa-confirm');
    const label = ready ? t('confirm') : t('wait', { n: Math.max(1, Math.min(3, Math.ceil(3 - (time - 14.6)))) });
    if (confirm.textContent !== label) { confirm.textContent = label; fit(confirm); }
    confirm.classList.toggle('off', !ready);
    confirm.style.transform = `scale(${time >= 18.4 && time < 18.7 ? .96 : 1})`;

    const pop = eo(prog(time, 20.3, 20.9));
    $('xa-ok').style.transform = `scale(${.4 + .6 * pop})`; $('xa-ok').style.opacity = pop;
    const q = eo(prog(time, 21.2, 21.8));
    $('xa-queue').style.transform = `scale(${1.25 - .25 * q})`; $('xa-queue').style.opacity = q;
    ['xa-cash', 'xa-pass'].forEach((id, i) => {
      const p = eo(prog(time, 22.6 + i * .5, 23.2 + i * .5));
      $(id).style.opacity = p; $(id).style.transform = `translateY(${(1 - p) * 30}px)`;
    });

    const completed = time < 28.4 ? 0 : time < 29.8 ? 1 : time < 31.2 ? 2 : 3;
    for (let i = 0; i < 3; i++) {
      const el = $('xa-d' + i);
      el.className = 'xa-dot' + (i < completed ? ' fin' : i === completed ? ' on' : '');
      el.textContent = i < completed ? '✓' : String(i + 1);
    }
    const rows = root.querySelectorAll('.xa-trow');
    const top = rows[0].offsetTop + rows[0].offsetHeight / 2, bottom = rows[2].offsetTop + rows[2].offsetHeight / 2;
    $('xa-bar').style.top = top + 'px'; $('xa-bar').style.height = (bottom - top) + 'px';
    $('xa-barfill').style.height = eio(prog(time, 28.4, 31.2)) * 100 + '%';
    $('xa-receipt').style.opacity = eo(prog(time, 31.2, 31.8));

    const hint = $('xa-hint');
    hint.style.opacity = 1 - Math.min(prog(time, 32.3, 32.7), 1 - prog(time, 35.7, 36));
    hint.style.scale = String(1 + .035 * Math.sin(time * Math.PI * 1.6));
    [['xa-r1', 0], ['xa-r2', .6]].forEach(([id, off]) => {
      const p = ((time - 32.5 + off) % 1.2 + 1.2) % 1.2 / 1.2;
      $(id).style.opacity = time >= 32.5 ? (1 - p) * .8 : 0;
      $(id).style.transform = `scale(${.45 + p * .6})`;
    });

    // Finger cursor: glides to each target, presses, ripples.
    const cur = $('xa-cursor'), rip = $('xa-ripple');
    const shown = Math.max(...[[5.6, 12.6], [17.3, 19.2]].map(([a, b]) => Math.min(prog(time, a, a + .3), 1 - prog(time, b - .3, b))));
    cur.style.opacity = shown;
    let pos = null, pressed = 1, ripple = null;
    TAPS.forEach(([tt, id], i) => {
      if (time < tt - .7 || (i < TAPS.length - 1 && time >= TAPS[i + 1][0] - .7)) return;
      const target = center(id);
      const from = i > 0 && TAPS[i - 1][0] > tt - 3 ? center(TAPS[i - 1][1]) : [target[0] + 160, target[1] + 220];
      const m = eio(prog(time, tt - .7, tt - .1));
      pos = [from[0] + (target[0] - from[0]) * m, from[1] + (target[1] - from[1]) * m];
      if (time >= tt - .1 && time < tt + .2) pressed = .82;
      if (time >= tt && time < tt + .6) ripple = [target, prog(time, tt, tt + .6)];
    });
    pos ||= center('xa-usd');
    cur.style.left = pos[0] + 'px'; cur.style.top = pos[1] + 'px'; cur.style.transform = `scale(${pressed})`;
    if (ripple && shown > 0) {
      rip.style.left = ripple[0][0] + 'px'; rip.style.top = ripple[0][1] + 'px';
      rip.style.opacity = (1 - ripple[1]) * .9; rip.style.transform = `scale(${1 + ripple[1] * 1.6})`;
    } else rip.style.opacity = 0;
  }

  function frame(now) {
    const elapsed = (now - t0) / 1000, n = Math.floor(elapsed / DURATION);
    if (n !== loop) { loop = n; setLanguage(n); }
    render(elapsed - n * DURATION);
    raf = requestAnimationFrame(frame);
  }

  function show() {
    if (!root) build();
    if (!root.hidden) return;
    order = languages();
    root.hidden = false;
    loop = -1; t0 = performance.now();
    layout();
    raf = requestAnimationFrame(frame);
  }
  function hide() {
    if (!root || root.hidden) return;
    cancelAnimationFrame(raf); raf = 0;
    root.hidden = true;
  }
  function dismiss() {
    hide();
    dispatchEvent(new Event('kiosk-attract-dismiss'));
  }

  window.KioskAttract = {
    show, hide,
    get active() { return !!root && !root.hidden; },
    // For still frames and video capture: a fixed time (s) and language, without the clock.
    renderAt(time, code) {
      if (!root) build();
      order = languages(); root.hidden = false; cancelAnimationFrame(raf);
      const i = Math.max(0, order.indexOf(code));
      layout(); setLanguage(i); render(((time % DURATION) + DURATION) % DURATION);
    }
  };
})();
