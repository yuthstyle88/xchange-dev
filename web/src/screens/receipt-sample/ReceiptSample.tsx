import type { JSX } from 'solid-js';
import { AppShell } from '../../shared/ui/AppShell';
import { screenT } from '../../shared/i18n';
import { th, en } from './i18n';
import logoUrl from './logo.png';
import qrUrl from './qr-telegram.png';
import './receipt.css';
import './print.css';

export function ReceiptSample(props: { print?: () => void }): JSX.Element {
  const t = screenT('receipt-sample', { th, en });

  const handlePrint = () => {
    if (props.print) {
      props.print();
    } else {
      window.print();
    }
  };

  return (
    <AppShell title="XCHANGE • Exchange Ticket" ownsMain>
      <p class="screen-heading">XCHANGE / EXCHANGE TICKET</p>
      <main class="receipt" aria-label="Exchange transaction receipt">
        <section class="main-ticket">
          <p style="font-size:8px;text-align:center;font-weight:bold;border:1px dashed;padding:3px;margin:1mm 0">{t('banner')}</p>
          <header class="brand">
            <div class="logo-window"><img class="logo" alt="XCHANGE Currency Exchange" src={logoUrl} /></div>
            <div class="ticket-label"><strong>EXCHANGE</strong>TRANSACTION<br />RECEIPT / 01</div>
          </header>
          <div class="route" aria-hidden="true">
            <span class="route-code">USD</span>
            <div class="route-mid"><svg viewBox="0 0 100 20" fill="none"><path d="M0 10H91M85 4L93 10L85 16" stroke="currentColor" stroke-width="1" stroke-dasharray="3 3" /></svg>CURRENCY EXCHANGE</div>
            <span class="route-code">THB</span>
          </div>
          <dl>
            <div class="row receipt-number"><dt>Receipt No.</dt><dd>EX202609270001</dd></div>
            <div class="row"><dt>Type</dt><dd>SELL</dd></div>
            <div class="row passenger"><dt>Customer</dt><dd>John Smith</dd></div>
            <div class="row"><dt>Currency</dt><dd>USD</dd></div>
            <div class="row"><dt>Amount</dt><dd>1,000.00 USD</dd></div>
            <div class="row"><dt>Booked Rate</dt><dd>33.2500</dd></div>
            <div class="row"><dt>Time</dt><dd>27/09/2026 14:05</dd></div>
            <div class="row"><dt>Customer ID</dt><dd>CX-20260927-00001</dd></div>
            <div class="row"><dt>Pay Via</dt><dd>Cash</dd></div>
            <div class="row"><dt>Ref No.</dt><dd>—</dd></div>
            <div class="row"><dt>Staff</dt><dd>ADMIN01</dd></div>
          </dl>
          <dl id="settlement">
            <div class="row"><dt>Customer Fee THB</dt><dd>0.00</dd></div>
            <div class="row"><dt>Net Received THB</dt><dd>33,250.00</dd></div>
          </dl>
          <dl>
            <div class="row total"><dt>THB Total</dt><dd>33,250.00<span class="unit"> THB</span></dd></div>
          </dl>
        </section>
        <div class="perforation" aria-hidden="true"><i class="notch left"></i><span>KEEP EXPLORING</span><i class="notch right"></i></div>
        <footer class="stub">
          <h2 class="promo">USDT REAL-TIME RATE UPDATE<br />&amp; RATE RESERVATION</h2>
          <div class="footer-grid">
            <a href="https://t.me/XCHANGEXTK" aria-label="Open Telegram"><img class="qr" alt="Scan to open Telegram @XCHANGEXTK" src={qrUrl} /></a>
            <div>
              <div class="scan-copy"><b>Scan QR Code for</b><br />Live Exchange Rates<br />&amp; Rate Reservation</div>
              <div class="contacts">WhatsApp<br /><a href="https://wa.me/66968799989"><strong>(+66) 96 879 9989</strong></a><br />Telegram: <a href="https://t.me/XCHANGEXTK"><strong>@XCHANGEXTK</strong></a></div>
            </div>
          </div>
          <div class="company"><strong>T.K. EXCHANGE MONEY CO., LTD.</strong>REG. 0205567035266 &middot; LIC. MC125670033<br />HEAD OFFICE: 75/223 MOO 12, SOI JOMTIEN 7<br />NONG PRUE, BANG LAMUNG, CHON BURI, THAILAND</div>
        </footer>
      </main>
      <button class="print-button" onClick={handlePrint}>Print receipt &middot; 80 × 90 mm</button>
      <p class="print-tip">NEOCAL &middot; 80 × 90 mm paper &middot; Scale 100%<br />Turn off browser headers and footers.</p>
    </AppShell>
  );
}
