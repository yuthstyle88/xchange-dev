// Customer kiosk. Texts come from kiosk-i18n.js; the language starts from the
// device's language and returns to it whenever the screen resets.
// xchange: no `?branch=` no longer means MAIN. A paired branch tablet takes its own branch; a plain
// web visit chooses one in Step 1 (#branch-choice), and an inactivity reset clears that choice
// (owner 2026-11-05: "มี3 สเต็ป"). `fixedBranch` is the branch no reset clears.
const $=id=>document.getElementById(id),urlBranch=new URLSearchParams(location.search).get('branch')||'';
let branch=urlBranch,fixedBranch=urlBranch,branches=[];
// xchange: the owner-set values (quick amounts, tiles, idle reset, review hold, refresh), never constants
// here (owner 2026-11-05: "ว่า อันไหน คอนฟิก ได้ อย่า ฝังใหน code"). The page carries the "not set" ones
// (api/src/application/kiosk_screen.rs); every `rates` answer carries the branch's own as `screen`.
// NOT_SET_FALLBACK is only for a server too old to send them: the same "not set" values, field by field,
// kept equal to kiosk_screen.rs by api/tests/pages_kiosk_test.rs. The server's values always win.
const NOT_SET_FALLBACK={quickAmounts:[100,500,1000,5000],tiles:6,idleSeconds:120,reviewSeconds:3,refreshSeconds:10,defaultPaymentMethod:"Cash"};
let kioskValues={...NOT_SET_FALLBACK,...window.XC_KIOSK_SCREEN};
const LANGS=window.KIOSK_LANGS,TEXT=window.KIOSK_TEXT,ERRORS=window.KIOSK_ERRORS;
let touchAmount='';
let lastRatesAt=0,ratesLoading=false,rateGeneration=0;
let model,type='BUY',quote,lastStatus,timer,seconds=0,idle,successReset,busy=false,screen='select';
// Phone mode: a customer who scanned the rate board QR (?via=qr) queues from their own phone, so the
// queue survives a reload, the screen never resets on idle, and the phone alerts when staff take the queue.
const personal=new URLSearchParams(location.search).get('via')==='qr',MINE='xc.kiosk.queue.'+branch,baseTitle=document.title;
const remember=()=>{try{localStorage.setItem(MINE,JSON.stringify({quote,at:Date.now()}));}catch{}},forget=()=>{try{localStorage.removeItem(MINE);}catch{}};
let audio,alerted='';
function prepareAlert(){try{audio??=new (window.AudioContext||window.webkitAudioContext)();audio.resume?.();}catch{}}
function alertTurn(){try{navigator.vibrate?.([400,200,400,200,800]);}catch{}
 try{if(!audio)return;const now=audio.currentTime;[880,1175,1568,1175,1568].forEach((f,i)=>{const o=audio.createOscillator(),g=audio.createGain(),at=now+i*.24;o.frequency.value=f;o.connect(g);g.connect(audio.destination);g.gain.setValueAtTime(.0001,at);g.gain.exponentialRampToValueAtTime(.35,at+.02);g.gain.exponentialRampToValueAtTime(.0001,at+.22);o.start(at);o.stop(at+.24);});}catch{}}
const fmt=n=>Number(n).toLocaleString('en-US',{minimumFractionDigits:2,maximumFractionDigits:2});

function detectLang(){for(const tag of navigator.languages||[navigator.language||'en']){const t=String(tag).toLowerCase();if(/^zh-(hk|mo|tw)|^zh-hant/.test(t))return 'zh-HK';const hit=LANGS.find(l=>l.code===t.split('-')[0]);if(hit)return hit.code;}return 'en';}
const defaultLang=detectLang();
let lang=defaultLang,info,currencyNames,regionNames;
const t=(key,vars={})=>String((TEXT[lang]||{})[key]??TEXT.en[key]??key).replace(/\{(\w+)\}/g,(_,k)=>vars[k]??'');
const errText=msg=>ERRORS[msg]?t(ERRORS[msg]):t('errGeneric');
function displayNames(type){try{return new Intl.DisplayNames([info.lang,'en'],{type});}catch{return null;}}
// Flags identify the currency issuer, not an arbitrary country that accepts it.
function currencyFlag(code){
 const issuers={USD:'US',EUR:'EU',GBP:'GB',THB:'TH',SGD:'SG',CNY:'CN',JPY:'JP',KRW:'KR',HKD:'HK',TWD:'TW',AUD:'AU',CAD:'CA',NZD:'NZ',CHF:'CH',AED:'AE',SAR:'SA',INR:'IN',MYR:'MY',IDR:'ID',VND:'VN',PHP:'PH',RUB:'RU',ZAR:'ZA',NOK:'NO',DKK:'DK'};
 const countries=model?.rates.find(r=>r.code===code)?.countries;
 const country=issuers[code]||(countries?.length===1?countries[0].code:'');
 return /^[A-Z]{2}$/.test(country)?String.fromCodePoint(...[...country].map(c=>127397+c.charCodeAt(0))):'🌐';
}
function currencyName(r){const n=currencyNames?.of(r.code);return n&&n!==r.code?n:(lang==='th'?r.name:r.nameEn)||r.code;}
// Amounts and codes stay left-to-right inside right-to-left text.
function ltr(el){el.dir='ltr';return el;}

function setLang(code){
 lang=TEXT[code]?code:'en';info=LANGS.find(l=>l.code===lang);
 document.documentElement.lang=info.lang;document.documentElement.dir=info.dir||'ltr';
 currencyNames=displayNames('currency');regionNames=displayNames('region');
 for(const el of document.querySelectorAll('[data-i18n]'))el.textContent=t(el.dataset.i18n);
 for(const el of document.querySelectorAll('[data-i18n-placeholder]')){el.placeholder=t(el.dataset.i18nPlaceholder);el.setAttribute('aria-label',el.placeholder);}
 $('lang').value=lang;$('lang').setAttribute('aria-label',t('lang'));
 if(!model)$('branch').textContent=t('welcome');
 show(screen,true);filterCurrencies();renderLanguageOptions();renderBranchOptions();
 if(quote){renderQuote('quote');renderQuote('done-summary');renderIdentity(lastStatus||quote,screen==='done'?'done-identity':'review-identity');}
 if(lastStatus&&screen==='done')renderProgress(lastStatus);
 if(screen==='confirm')countdownLabel();
}
$('lang').replaceChildren(...LANGS.map(l=>{const o=new Option(l.name,l.code);o.lang=l.lang;return o;}));
$('lang').onchange=()=>setLang($('lang').value);

// xchange: our /api/kiosk refuses a call without `X-Xchange-Request: 1` (origin.rs same_origin); keep it.
async function api(action,payload={}){const r=await fetch('/api/kiosk',{method:'POST',signal:AbortSignal.timeout(10000),headers:{'Content-Type':'application/json','X-Xchange-Request':'1'},body:JSON.stringify({action,payload:{...payload,branchId:branch}})});const d=await r.json();if(!r.ok)throw Error(d.error);return d;}
function show(name,keepError){if(!keepError)window.scrollTo({top:0,behavior:'instant'});screen=name;for(const id of ['select','confirm','done'])$(id).hidden=id!==name;$('progress').textContent=t(name==='select'?'step1':name==='confirm'?'step2':'step3');if(!keepError)$('error').textContent='';}
function rate(){return model?.rates.find(r=>r.code===$('currency').value);}
function symbol(code){if(!code)return '';return new Intl.NumberFormat('en-US',{style:'currency',currency:code,currencyDisplay:'narrowSymbol'}).formatToParts(0).find(p=>p.type==='currency')?.value||code;}
function refresh(){
 const fresh=lastRatesAt>0&&performance.now()-lastRatesAt<30000;
 const r=fresh?rate():null,buy=type==='BUY',code=$('currency').value,key=buy?'buy':'sell',v=Number(r?.[key]);
 $('rates').replaceChildren();const d=document.createElement('div'),b=ltr(document.createElement('strong'));
 d.textContent=t(buy?'weBuy':'weSell');b.textContent=v>0?`1 ${symbol(code)} ${code} = ${v.toLocaleString('en-US',{minimumFractionDigits:4,maximumFractionDigits:5,useGrouping:false})} ฿ THB`:t('notAvailable');d.append(b);$('rates').append(d);
 for(const o of $('payment-method').options){const rule=model?.paymentMethods?.find(m=>m.id===o.value);o.disabled=!o.value||!rule?.enabled||!rule[buy?'buy':'sell'];o.hidden=!!o.value&&o.disabled;}if(!$('payment-method').selectedOptions[0]||$('payment-method').selectedOptions[0].disabled)pickPayment(); // xchange: never an empty choice
 const give=buy?code:'THB',receive=buy?'THB':code;
 $('amount-label').textContent=t('amountLabel',{code:currencyFlag(give)+' '+give});
 $('amount-code').textContent=give; // xchange: the code inside the amount field on a phone (kiosk-xchange.css)
 $('receive-label').textContent=t('youReceive')+' ('+currencyFlag(receive)+' '+receive+')';
 const amount=Number($('amount').value),valid=Number.isFinite(amount)&&amount>0&&amount<=10000000&&v>0;
 const foreign=buy?Math.round(amount*100)/100:Math.floor(amount/v*100)/100;
 const total=buy?Math.round(foreign*v*100)/100:foreign;
 $('receive').value=valid?symbol(receive)+' '+fmt(total)+' '+receive:'';
 $('estimate').textContent=valid?t('estimate',{amount:fmt(total)+' '+receive}):'';
 let note=$('payment-instruction');if(!note){note=document.createElement('p');note.id='payment-instruction';note.className='small';note.setAttribute('role','status');$('payment-method').after(note);}
 const selected=model?.paymentMethods?.find(m=>m.id===$('payment-method').value);
 const displayOnly=r?.quoteEnabled===false;
 // xchange: no method at all for this side: their worded "contact staff" state instead of an empty select
 const noMethod=!!model&&![...$('payment-method').options].some(o=>!o.disabled);$('payment-method').hidden=noMethod;
 note.textContent=displayOnly||noMethod?t('contactCounter'):selected?.instruction||'';
 $('review').textContent=t(displayOnly?'contactCounter':'review');
 // xchange: an empty or invalid amount no longer disables Review; tapping it words the error instead
 // (owner 2026-11-05: no English browser bubble). Everything else still disables it as before.
 $('review').disabled=displayOnly||busy||!fresh||!model?.open||!(v>0)||!selected?.enabled||!selected[buy?'buy':'sell'];
}
async function reset(){
 rateGeneration++;touchAmount='';document.title=baseTitle;lastRatesAt=0;model=null;$('customer-receipt')?.remove();
 clearTimeout(successReset);successReset=null;clearInterval(timer);quote=null;lastStatus=null;type='BUY';$('language-search').value='';$('payment-method').value='';$('amount').value='';
 $('buy').classList.add('selected');$('sell').classList.remove('selected');$('buy').setAttribute('aria-pressed','true');$('sell').setAttribute('aria-pressed','false');
 if(!fixedBranch){branch='';$('branch-search').value='';} // xchange: a picked branch is the last customer's
 markQuickAmount(''); // xchange: no quick amount is chosen on a fresh screen
 screen='select';setLang(defaultLang);show('select');
 $('currency-search').value='';await loadRates();
}
for(const [id,value] of [['buy','BUY'],['sell','SELL']])$(id).onclick=()=>{type=value;for(const name of ['buy','sell']){$(name).classList.toggle('selected',name===id);$(name).setAttribute('aria-pressed',String(name===id));}refresh();pickPayment();refresh();}; // xchange: a new side takes its default method
$('amount').oninput=refresh;$('currency').onchange=refresh;

function renderQuote(target){
 const box=$(target);box.replaceChildren();if(!quote)return;
 const give=quote.type==='BUY'?[quote.foreign,quote.currency]:[quote.thb,'THB'],get=quote.type==='BUY'?[quote.thb,'THB']:[quote.foreign,quote.currency];
 for(const [label,value] of [['youGive',fmt(give[0])+' '+give[1]],['payment',paymentName(quote.paymentMethod||'Cash')],['rate',`1 ${quote.currency} = ${quote.rate.toLocaleString('en-US',{minimumFractionDigits:4,maximumFractionDigits:5,useGrouping:false})} THB`],['youReceive',fmt(get[0])+' '+get[1]]]){const row=document.createElement('div');const k=document.createElement('span'),v=ltr(document.createElement('strong'));k.textContent=t(label);v.textContent=value;row.append(k,v);box.append(row);}
}
function countdownLabel(){$('confirm-button').textContent=seconds>0?t('wait',{n:seconds}):t('confirm');}
$('form').onsubmit=async e=>{e.preventDefault();if(busy||$('review').disabled)return;
 if(!amountReady()){$('error').textContent=t('errAmount');return;} // xchange: the worded error, in the screen's language
 busy=true;$('review').disabled=true;
 try{quote=await api('quote',{type,currency:$('currency').value,amount:$('amount').value,paymentMethod:$('payment-method').value});lastStatus=null;show('confirm');renderIdentity(quote,'review-identity');renderQuote('quote');
  seconds=kioskValues.reviewSeconds??0;$('confirm-button').disabled=true; // xchange: owner-set
 countdownLabel();clearInterval(timer);
  timer=setInterval(()=>{seconds--;countdownLabel();if(seconds<=0){clearInterval(timer);$('confirm-button').disabled=false;}},1000);}
 catch(err){$('error').textContent=errText(err.message);}finally{busy=false;refresh();}};
$('back').onclick=()=>{clearInterval(timer);quote=null;lastStatus=null;show('select');};
$('confirm-button').onclick=async()=>{if(busy||!quote)return;busy=true;$('confirm-button').disabled=true;prepareAlert();
 try{const d=await api('confirm',{id:quote.id});lastStatus=d;if(personal)remember();$('queue').textContent=d.queue;show('done');renderIdentity(d,'done-identity');renderProgress(d);renderQuote('done-summary');}
 catch(e){$('error').textContent=errText(e.message);$('confirm-button').disabled=false;}finally{busy=false;}};
// After 2 minutes without a touch the screen resets and the screensaver (kiosk-attract.js) starts;
// a tap on it returns to the start screen. It never covers a queue number on the done screen.
// xchange: the reset delay is the owner-set idleSeconds; with none the screen never resets by itself.
function activity(){if(personal)return;clearTimeout(idle);if(!(kioskValues.idleSeconds>0))return;idle=setTimeout(()=>{if(!busy&&$('done').hidden){reset();window.KioskAttract?.show();}activity();},kioskValues.idleSeconds*1000);}
addEventListener('kiosk-attract-dismiss',()=>{if(!busy)reset();activity();});
document.addEventListener('pointerdown',activity);document.addEventListener('keydown',activity);activity();
if(personal)document.addEventListener('pointerdown',prepareAlert);
// xchange: the branch is settled (URL, paired tablet, or the Step 1 control) before the first load.
settleBranch().then(reset).then(restore);

function filterCurrencies(){
 const selected=$('currency').value,q=$('currency-search').value.trim().toLocaleLowerCase();$('currency').replaceChildren();
 for(const r of model?.rates||[]){
  const text=[r.code,r.name,r.nameEn,currencyName(r),...(r.countries||[]).flatMap(c=>[c.code,c.en,c.th,regionNames?.of(c.code)])].join(' ').toLocaleLowerCase();
  // Top-demand currencies the owner highlights on /pricing come first in the feed and get a star.
  if(!q||text.includes(q))$('currency').add(new Option((r.featured?'★ ':'')+currencyFlag(r.code)+' '+r.code,r.code));
 }
 if([...$('currency').options].some(o=>o.value===selected))$('currency').value=selected;refresh();
 if(model&&!$('currency').options.length)$('estimate').textContent=t('noCurrencies');
}
$('currency-search').oninput=filterCurrencies;

function renderIdentity(data,target){
 const box=$(target);box.replaceChildren();
 for(const [title,value] of [['orderNo',data.orderNo],['transactionNo',data.transactionNo||t('notCreated')],['status',TEXT.en['st'+data.status]?t('st'+data.status):data.status]]){
  const row=document.createElement('div'),label=document.createElement('span'),text=document.createElement('strong');label.textContent=t(title);text.textContent=value;if(title!=='status')ltr(text);row.append(label,text);box.append(row);
 }
}
let checkingStatus=false;
async function checkStatus(){if(!quote||checkingStatus||document.hidden||!$('select').hidden)return;checkingStatus=true;const id=quote.id;
 try{const d=await api('status',{id});if(quote?.id===id){lastStatus=d;renderIdentity(d,$('done').hidden?'review-identity':'done-identity');if(!$('done').hidden)renderProgress(d);
  if(d.status==='EXPIRED'&&!$('confirm').hidden){clearInterval(timer);$('confirm-button').disabled=true;$('error').textContent=t('quoteExpired');}}}
 catch(e){if(quote?.id===id)$('error').textContent=t('statusError');}finally{checkingStatus=false;}}
setInterval(checkStatus,5000);document.addEventListener('visibilitychange',checkStatus);

function renderProgress(data){
 renderTurn(data);
 if(data.status==='COMPLETED'&&!successReset&&!personal){const id=quote?.id;successReset=setTimeout(()=>{if(quote?.id===id)reset();},30000);}
 const stage={WAITING:-1,ACCEPTED:0,ID_CHECKED:1,COMPLETED:2}[data.status]??-1;
 [...$('order-progress').children].forEach((el,i)=>{el.classList.toggle('active',i===stage);el.classList.toggle('finished',i<stage);if(i===stage)el.setAttribute('aria-current','step');else el.removeAttribute('aria-current');});
 $('phase-message').textContent=t(stage<0?'phaseWaiting':stage===0?'phasePassport':stage===1?'phaseProcessing':'phaseDone');
}

// Queue position while waiting, then "your turn" once staff take the queue (ACCEPTED).
function renderTurn(data){
 const box=$('queue-ahead'),turn=data.status==='ACCEPTED',waiting=data.status==='WAITING'&&Number.isInteger(data.ahead);
 box.hidden=!turn&&!waiting;box.classList.toggle('turn',turn);
 box.textContent=turn?t('yourTurn'):!waiting?'':data.ahead>0?t('queueAhead',{n:data.ahead}):t('queueNext');
 $('keep-open').hidden=!personal||!['WAITING','ACCEPTED'].includes(data.status);
 $('next-customer').hidden=personal&&data.status!=='COMPLETED';
 if(personal){document.title=turn?'🔔 '+t('yourTurn'):baseTitle;if(turn&&alerted!==data.id){alerted=data.id;alertTurn();}}
}
// Phone mode: bring back the queue this phone made (same branch, within 12 hours) after a reload.
async function restore(){
 if(!personal)return;let saved;try{saved=JSON.parse(localStorage.getItem(MINE)||'null');}catch{}
 if(!saved?.quote?.id||Date.now()-saved.at>43200000){forget();return;}
 try{const d=await api('status',{id:saved.quote.id});if(!['WAITING','ACCEPTED','ID_CHECKED','COMPLETED'].includes(d.status)){forget();return;}
  quote=saved.quote;lastStatus=d;$('queue').textContent=d.queue;show('done');renderIdentity(d,'done-identity');renderProgress(d);renderQuote('done-summary');}
 catch{}
}
function paymentName(value){return t({'Credit Card':'creditCard',Cash:'cash','Bank Transfer':'transfer'}[value]||value);}
function renderLanguageOptions(){
 const q=$('language-search').value.trim().toLocaleLowerCase();
 const matches=LANGS.filter(l=>[l.name,l.code,l.search].join(' ').toLocaleLowerCase().includes(q));
 $('lang').replaceChildren(...matches.map(l=>new Option(l.name+(l.rank?' · #'+l.rank:''),l.code)));
 if(matches.some(l=>l.code===lang))$('lang').value=lang;else $('lang').selectedIndex=-1;
}
$('language-search').oninput=renderLanguageOptions;
let receiptLoading=false;
setInterval(async()=>{
  const existing=document.getElementById('customer-receipt');
  if(!quote||$('done').hidden){existing?.remove();return;}
  if(receiptLoading||document.hidden)return;
  receiptLoading=true;const id=quote.id;
  try{
    const d=await api('customerReceipt',{id});if(quote?.id!==id)return;
    if(!d.image){existing?.remove();return;}
    let box=existing;
    if(!box){box=document.createElement('section');box.id='customer-receipt';box.innerHTML='<h2>Your receipt · ใบเสร็จของคุณ</h2><p></p><img alt="Exchange receipt" style="display:block;width:100%;max-width:400px;margin:16px auto"><a style="display:inline-block;padding:16px;background:#ab0044;color:white;border-radius:12px">Save image · บันทึกเป็นรูปภาพ</a>';$('done').append(box);}
    box.querySelector('h2').textContent=t('receiptTitle');box.querySelector('a').textContent=t('saveImage');box.querySelector('p').textContent=t(d.status==='COMPLETED'?'stCOMPLETED':'stID_CHECKED');
    box.querySelector('img').src=d.image;
    const link=box.querySelector('a');link.href=d.image;link.download=(d.transactionNo||d.queue+'_PENDING')+'_RECEIPT.png';
  }catch(e){/* Keep the last received image during a brief connection failure. */}
  finally{receiptLoading=false;}
},3000);


function renderCurrencyShortcuts(){
 const box=$('currency-shortcuts');if(!box)return;box.replaceChildren();
 for(const option of tileOptions()){
   const currency=model?.rates.find(r=>r.code===option.value);
   const button=document.createElement('button');button.type='button';
   const flag=document.createElement('span');flag.className='currency-flag';flag.setAttribute('aria-hidden','true');flag.textContent=currencyFlag(option.value);
   const code=ltr(document.createElement('bdi'));code.textContent=option.value;
   // xchange: a phone shows the round symbol icon of the attract demo card instead of the flag (kiosk-xchange.css).
   const sym=document.createElement('span');sym.className='currency-sym';sym.setAttribute('aria-hidden','true');sym.textContent=symbol(option.value);
   button.append(flag,sym,code);
   button.setAttribute('aria-label',option.value+' · '+(currency?currencyName(currency):option.value));
   if(currency?.featured){button.classList.add('featured');button.dataset.rank=currency.rank;}
   if(currency?.quoteEnabled===false){const hint=document.createElement('small');hint.textContent=t('contactCounter');button.append(hint);}
   button.classList.toggle('selected',option.selected);button.setAttribute('aria-pressed',String(option.selected));
   button.onclick=()=>{$('currency').value=option.value;refresh();};box.append(button);
 }
}
const refreshBeforeShortcuts=refresh;
refresh=function(){refreshBeforeShortcuts();renderCurrencyShortcuts();};
renderCurrencyShortcuts();

$('payment-method').addEventListener('change',refresh);

// xchange: their loadRates also read GET /api/rate-board?branch=, which our API does not have and
// will not add (docs/design/blueprint/rate-display.md:314). Our kiosk `rates` answer alone carries
// branchId, branchName, open and rates (the branch's SHOW currencies, quotable), and `ask` (its ASK
// currencies, no prices: listed with "Please contact staff", never quoted; quote.rs refuses ASK off the
// desk). HIDE ones are in neither. The list follows the answer, so a currency the admin turns on appears on
// the next refresh. Quotes are still checked again by the server.
async function loadRates(){
 const generation=rateGeneration;
 if(!branch){model=null;lastRatesAt=0;if(screen==='select'){filterCurrencies();refresh();}return;} // xchange: no branch chosen yet
 try{
  const settings=await api('rates');
  if(!Array.isArray(settings.rates))throw Error('feed');
  if(generation!==rateGeneration)return;
  model={...settings,rates:[...settings.rates.map(r=>({...r,quoteEnabled:true})),...(settings.ask||[]).map(r=>({...r,quoteEnabled:false}))]};
  if(settings.screen){kioskValues={...kioskValues,...settings.screen};renderQuickAmounts();} // xchange: the branch's owner-set values
  if(!$('payment-method').value)pickPayment(); // xchange: preselect on load
  lastRatesAt=performance.now();$('branch').textContent=settings.branchName;
  if(screen==='select'){filterCurrencies();$('error').textContent=model.open?'':t('closed');}
 }catch(e){if(generation===rateGeneration&&screen==='select'){$('error').textContent=t('errGeneric');refresh();}}
}
// xchange: the refresh period is the owner-set refreshSeconds, read again before every wait.
(function pollRates(){if(!(kioskValues.refreshSeconds>0))return;setTimeout(async()=>{
 if(!(screen!=='select'||busy||ratesLoading||document.hidden)){ratesLoading=true;try{await loadRates();}finally{ratesLoading=false;}}
 pollRates();
},kioskValues.refreshSeconds*1000);})();
setInterval(()=>{if(screen==='select'&&lastRatesAt&&performance.now()-lastRatesAt>=30000)refresh();},1000);
$('next-customer').onclick=()=>{if(!busy){forget();reset();}};
// An on-screen keypad works on touch displays without a system keyboard.
$('amount').addEventListener('input',()=>{touchAmount=$('amount').value;});
for(const key of ['1','2','3','4','5','6','7','8','9','.','0','⌫']){
 const button=document.createElement('button');button.type='button';button.textContent=key;
 button.onclick=()=>{
  if(!touchAmount.startsWith($('amount').value)||Number(touchAmount)!==Number($('amount').value))touchAmount=$('amount').value;
  const next=key==='⌫'?touchAmount.slice(0,-1):key==='.'?(touchAmount.includes('.')?touchAmount:(touchAmount||'0')+'.'):touchAmount+key;
  if(!/^\d{0,8}(\.\d{0,2})?$/.test(next)||Number(next)>10000000)return;
  touchAmount=next;$('amount').value=next.endsWith('.')?next.slice(0,-1):next;refresh();
 };
 $('amount-keypad').append(button);
}

// Bind through the final renderer so dropdown changes update touch selection too.
$('currency').onchange=()=>refresh();
$('amount').oninput=()=>refresh();

// xchange: the branch control of a plain web visit (owner 2026-11-05: "มี3 สเต็ป"), built like
// #language-search + #lang. The list is the public one our customer pages read
// (GET /api/public/host, then GET /api/public/orgs/{org}/branches). A paired tablet
// (GET /api/branch-tablet/me) or a ?branch= hides it. Review stays disabled until a branch is
// chosen, because no rates load without one.
async function getJson(url){const r=await fetch(url,{credentials:'same-origin',cache:'no-store',signal:AbortSignal.timeout(7000)});if(!r.ok)throw Error('feed');return r.json();}
async function settleBranch(){
 if(!fixedBranch){try{const me=await getJson('/api/branch-tablet/me');if(me?.paired===true&&me.branchCode){fixedBranch=me.branchCode;branch=fixedBranch;}}catch{}}
 $('branch-choice').hidden=!!fixedBranch;
 if(fixedBranch)return;
 try{const host=await getJson('/api/public/host');const list=await getJson('/api/public/orgs/'+encodeURIComponent(host.org)+'/branches');branches=Array.isArray(list?.branches)?list.branches:[];}
 catch{branches=[];}
 renderBranchOptions();
}
function renderBranchOptions(){
 const box=$('branch-select');if(!box||fixedBranch)return;
 const q=$('branch-search').value.trim().toLocaleLowerCase();
 const matches=branches.filter(b=>[b.name,b.code,b.id,b.address].join(' ').toLocaleLowerCase().includes(q));
 const none=new Option(t('chooseBranch'),'');
 box.replaceChildren(none,...matches.map(b=>new Option(b.name,b.id)));
 box.value=matches.some(b=>b.id===branch)?branch:'';
 box.setAttribute('aria-label',t('branchLabel'));
}
function chooseBranch(id){
 if(id===branch)return;
 rateGeneration++;branch=id;model=null;lastRatesAt=0;
 if(!branch)$('branch').textContent=t('welcome');
 filterCurrencies();loadRates();
}
$('branch-search').oninput=renderBranchOptions;
$('branch-select').onchange=()=>chooseBranch($('branch-select').value);

// xchange: quick amounts (owner 2026-11-05: "ให้มี 500 1000 2000 3000 แล้ว คีย์", "ถ้ากด อีกให้ เป็น เพิ่ม", values "100 500 1000 5000", labelled "+").
// On an empty or 0 field a chip sets the amount; every later tap adds its value to what the field holds,
// typed or tapped (100 + 100 = 200; 200 typed + 500 = 700). The keypad then goes on from it (it
// reads touchAmount); typing clears the highlight, which otherwise marks the chip tapped last.
// A sum past the field's own limits (10,000,000; 2 decimals) is not taken. Grouped as their fmt() (en-US).
function markQuickAmount(value){for(const b of $('quick-amounts').children){const on=b.dataset.amount===value;b.classList.toggle('selected',on);b.setAttribute('aria-pressed',String(on));}}
// The chips are the owner-set quickAmounts; drawn again only when the list changes.
function renderQuickAmounts(){
 const values=(Array.isArray(kioskValues.quickAmounts)?kioskValues.quickAmounts:[]).filter(v=>Number.isFinite(v)&&v>0);
 if(values.join()===[...$('quick-amounts').children].map(b=>b.dataset.amount).join())return;
 $('quick-amounts').replaceChildren(...values.map(value=>{
  const button=document.createElement('button');button.type='button';button.dataset.amount=String(value);
  button.textContent='+'+value.toLocaleString('en-US'); // owner: "มี เครื่องหมาย +"
  button.setAttribute('aria-pressed','false');
  button.onclick=()=>{
   const held=Number($('amount').value),next=String(Math.round(((Number.isFinite(held)&&held>0?held:0)+value)*100)/100);
   if(!/^\d{0,8}(\.\d{0,2})?$/.test(next)||Number(next)>10000000)return;
   touchAmount=next;$('amount').value=next;refresh();markQuickAmount(String(value));
  };
  return button;
 }));
}
renderQuickAmounts();
$('amount-keypad').addEventListener('click',()=>markQuickAmount(''));
$('amount').addEventListener('input',()=>markQuickAmount(''));

// xchange: what refresh() used to require of the amount before Review could be pressed.
function amountReady(){const r=rate(),v=Number(r?.[type==='BUY'?'buy':'sell']),amount=Number($('amount').value);
 if(!(Number.isFinite(amount)&&amount>0&&amount<=10000000&&v>0))return false;
 return (type==='BUY'?Math.round(amount*100)/100:Math.floor(amount/v*100)/100)>0;}

// xchange: the tiles (owner 2026-11-05: popular currencies from the branch's trading history). With no
// search, the branch's quotable (SHOW) currencies only: its popular ones first (`popular` of the rates
// answer, worked out on the server), then the rest in the answer's order, up to the owner-set `tiles`; the
// one selected always stays. A search shows every match, ASK ones with "contact staff", and the dropdown
// keeps every currency, as their kiosk does.
function tileOptions(){
 const options=[...$('currency').options];
 if($('currency-search').value.trim()||!(kioskValues.tiles>0))return options;
 const rank=code=>{const i=(model?.popular||[]).indexOf(code);return i<0?Infinity:i;};
 const quotable=o=>model?.rates.find(r=>r.code===o.value)?.quoteEnabled!==false;
 const ordered=options.filter(quotable).map((o,i)=>({o,i})).sort((a,b)=>rank(a.o.value)-rank(b.o.value)||a.i-b.i).map(x=>x.o);
 const shown=ordered.slice(0,kioskValues.tiles);
 const selected=options.find(o=>o.selected);
 if(selected&&!shown.includes(selected))shown.push(selected);
 return shown;
}

// xchange: the preselected payment method (owner 2026-11-05: "Payment method ให้ default"): the owner-set
// defaultPaymentMethod when this side allows it, else Cash, else the side's first enabled method in the
// answer's order; none when the side has none. The customer can still change it.
function pickPayment(){
 const select=$('payment-method'),side=type==='BUY'?'buy':'sell';
 const allowed=id=>{const m=model?.paymentMethods?.find(p=>p.id===id);return !!(m?.enabled&&m[side])&&[...select.options].some(o=>o.value===id);};
 const first=(model?.paymentMethods||[]).find(m=>allowed(m.id))?.id;
 const pick=[kioskValues.defaultPaymentMethod,'Cash',first].find(id=>id&&allowed(id));
 if(pick)select.value=pick;else select.selectedIndex=-1;
}
