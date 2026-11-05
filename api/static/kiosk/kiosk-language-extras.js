// Priority: 2025 full-year visitor source markets, shared languages deduplicated.
window.KIOSK_LANGS=[
{code:'ms',lang:'ms',name:'Bahasa Melayu',search:'Malay Malaysia มาเลเซีย มลายู',rank:1},
{code:'zh',lang:'zh-Hans',name:'简体中文',search:'Chinese China จีน 中国',rank:2},
{code:'hi',lang:'hi',name:'हिन्दी',search:'Hindi India อินเดีย ฮินดี भारत',rank:3},
{code:'ru',lang:'ru',name:'Русский',search:'Russian Russia รัสเซีย Россия',rank:4},
{code:'ko',lang:'ko',name:'한국어',search:'Korean South Korea เกาหลี 한국',rank:5},
{code:'ja',lang:'ja',name:'日本語',search:'Japanese Japan ญี่ปุ่น 日本',rank:6},
{code:'en',lang:'en',name:'English',search:'United Kingdom USA Singapore Australia อังกฤษ สหรัฐ สิงคโปร์ ออสเตรเลีย',rank:7},
{code:'zh-HK',lang:'zh-Hant',name:'繁體中文',search:'Taiwan Hong Kong Traditional Chinese ไต้หวัน ฮ่องกง 台灣 香港',rank:9},
{code:'de',lang:'de',name:'Deutsch',search:'German Germany เยอรมัน เยอรมนี',rank:11},
{code:'lo',lang:'lo',name:'ລາວ',search:'Lao Laos ลาว',rank:12},
{code:'fr',lang:'fr',name:'Français',search:'French France ฝรั่งเศส',rank:13},
{code:'id',lang:'id',name:'Bahasa Indonesia',search:'Indonesian Indonesia อินโดนีเซีย',rank:15},
{code:'my',lang:'my',name:'မြန်မာ',search:'Burmese Myanmar พม่า เมียนมา'},
{code:'vi',lang:'vi',name:'Tiếng Việt',search:'Vietnamese Vietnam เวียดนาม'},
{code:'ar',lang:'ar',dir:'rtl',name:'العربية',search:'Arabic Saudi Arabia UAE อาหรับ'},
{code:'th',lang:'th',name:'ไทย',search:'Thai Thailand ไทย'}
];
const paymentTexts={
en:['Payment method','Choose a method','Credit card','Cash','Bank transfer','If transfer, call agent','Your receipt','Save image'],
th:['ช่องทางชำระเงิน','เลือกช่องทาง','บัตรเครดิต','เงินสด','โอนธนาคาร','หากโอนเงิน กรุณาติดต่อพนักงาน','ใบเสร็จของคุณ','บันทึกเป็นรูปภาพ'],
ms:['Kaedah bayaran','Pilih kaedah','Kad kredit','Tunai','Pindahan bank','Untuk pindahan, hubungi kakitangan','Resit anda','Simpan imej'],
zh:['付款方式','选择方式','信用卡','现金','银行转账','如需转账，请联系工作人员','您的收据','保存图片'],
'zh-HK':['付款方式','選擇方式','信用卡','現金','銀行轉帳','如需轉帳，請聯絡工作人員','您的收據','儲存圖片'],
hi:['भुगतान का तरीका','तरीका चुनें','क्रेडिट कार्ड','नकद','बैंक ट्रांसफ़र','ट्रांसफ़र के लिए कर्मचारी से संपर्क करें','आपकी रसीद','चित्र सहेजें'],
ru:['Способ оплаты','Выберите способ','Кредитная карта','Наличные','Банковский перевод','Для перевода обратитесь к сотруднику','Ваш чек','Сохранить изображение'],
ko:['결제 방법','방법 선택','신용카드','현금','은행 송금','송금 시 직원에게 문의하세요','영수증','이미지 저장'],
ja:['お支払い方法','方法を選択','クレジットカード','現金','銀行振込','振込の場合はスタッフにお声がけください','領収書','画像を保存'],
de:['Zahlungsart','Bitte auswählen','Kreditkarte','Bargeld','Banküberweisung','Bei Überweisung Personal kontaktieren','Ihr Beleg','Bild speichern'],
lo:['ວິທີຊຳລະເງິນ','ເລືອກວິທີ','ບັດເຄຣດິດ','ເງິນສົດ','ໂອນທະນາຄານ','ຖ້າໂອນເງິນ ກະລຸນາຕິດຕໍ່ພະນັກງານ','ໃບຮັບເງິນຂອງທ່ານ','ບັນທຶກຮູບ'],
fr:['Mode de paiement','Choisir un mode','Carte de crédit','Espèces','Virement bancaire','Pour un virement, contactez un agent','Votre reçu','Enregistrer l’image'],
id:['Metode pembayaran','Pilih metode','Kartu kredit','Tunai','Transfer bank','Untuk transfer, hubungi petugas','Tanda terima Anda','Simpan gambar'],
my:['ငွေပေးချေမှုနည်းလမ်း','နည်းလမ်းရွေးပါ','ခရက်ဒစ်ကတ်','ငွေသား','ဘဏ်လွှဲခြင်း','ငွေလွှဲရန် ဝန်ထမ်းကို ဆက်သွယ်ပါ','သင့်ပြေစာ','ပုံသိမ်းရန်'],
vi:['Phương thức thanh toán','Chọn phương thức','Thẻ tín dụng','Tiền mặt','Chuyển khoản','Nếu chuyển khoản, hãy liên hệ nhân viên','Biên nhận của bạn','Lưu hình ảnh'],
ar:['طريقة الدفع','اختر الطريقة','بطاقة ائتمان','نقداً','تحويل مصرفي','للتحويل، تواصل مع الموظف','إيصالك','حفظ الصورة']
};
for(const [code,values] of Object.entries(paymentTexts))Object.assign(window.KIOSK_TEXT[code],Object.fromEntries(['payment','choosePayment','creditCard','cash','transfer','callAgent','receiptTitle','saveImage'].map((k,i)=>[k,values[i]])));

// Shared touch-screen handoff: the queue stays with staff when the next guest starts.
const walkInTexts={
 en:['Next customer','Keep your queue number before starting again. Your request remains with staff.'],
 th:['ลูกค้าคนถัดไป','กรุณาจดเลขคิวก่อนเริ่มใหม่ คำขอของคุณยังอยู่ในคิวของพนักงาน'],
 ms:['Pelanggan seterusnya','Simpan nombor giliran anda. Permintaan anda kekal dengan kakitangan.'],
 zh:['下一位顾客','请先记下排队号码。您的请求仍由工作人员处理。'],
 'zh-HK':['下一位顧客','請先記下排隊號碼。您的請求仍由工作人員處理。'],
 hi:['अगला ग्राहक','अपना कतार नंबर लिख लें। आपका अनुरोध कर्मचारियों के पास रहेगा।'],
 ru:['Следующий клиент','Сохраните номер очереди. Ваша заявка останется у сотрудников.'],
 ko:['다음 고객','대기 번호를 기억해 주세요. 요청은 직원에게 그대로 남아 있습니다.'],
 ja:['次のお客様','受付番号を控えてください。ご依頼はスタッフに引き継がれています。'],
 de:['Nächster Kunde','Notieren Sie Ihre Wartenummer. Ihre Anfrage bleibt beim Personal.'],
 lo:['ລູກຄ້າຄົນຕໍ່ໄປ','ກະລຸນາຈົດເລກຄິວໄວ້. ຄຳຂໍຂອງທ່ານຍັງຢູ່ກັບພະນັກງານ.'],
 fr:['Client suivant','Notez votre numéro de file. Votre demande reste auprès du personnel.'],
 id:['Pelanggan berikutnya','Simpan nomor antrean Anda. Permintaan Anda tetap ada pada petugas.'],
 my:['နောက်ဖောက်သည်','သင့်အလှည့်နံပါတ်ကို မှတ်ထားပါ။ သင့်တောင်းဆိုချက်သည် ဝန်ထမ်းထံတွင် ရှိနေပါမည်။'],
 vi:['Khách tiếp theo','Hãy ghi lại số thứ tự. Yêu cầu của bạn vẫn được nhân viên xử lý.'],
 ar:['العميل التالي','احتفظ برقم دورك. سيبقى طلبك لدى الموظفين.']
};
for(const [code,values] of Object.entries(walkInTexts))Object.assign(window.KIOSK_TEXT[code],{nextCustomer:values[0],keepQueue:values[1]});

// Screensaver (kiosk-attract.js): the touch prompt and the "example" tag on its sample figures.
const attractTexts={
 en:['Touch the screen to start','Example'],
 th:['แตะหน้าจอเพื่อเริ่ม','ตัวอย่าง'],
 ms:['Sentuh skrin untuk mula','Contoh'],
 zh:['触摸屏幕开始','示例'],
 'zh-HK':['輕觸螢幕開始','示例'],
 hi:['शुरू करने के लिए स्क्रीन छुएँ','उदाहरण'],
 ru:['Коснитесь экрана, чтобы начать','Пример'],
 ko:['화면을 터치하여 시작하세요','예시'],
 ja:['画面にタッチしてスタート','例'],
 de:['Bildschirm berühren, um zu starten','Beispiel'],
 lo:['ແຕະໜ້າຈໍເພື່ອເລີ່ມ','ຕົວຢ່າງ'],
 fr:['Touchez l’écran pour commencer','Exemple'],
 id:['Sentuh layar untuk memulai','Contoh'],
 my:['စတင်ရန် မျက်နှာပြင်ကို ထိပါ','ဥပမာ'],
 vi:['Chạm vào màn hình để bắt đầu','Ví dụ'],
 ar:['المس الشاشة للبدء','مثال']
};
for(const [code,values] of Object.entries(attractTexts))Object.assign(window.KIOSK_TEXT[code],{touchStart:values[0],example:values[1]});

// Queue texts for customers who queued from their own phone (QR on the rate board, ?via=qr) and
// the queue-ahead line on the done screen: [queueAhead, queueNext, yourTurn, keepOpen].
const queueTexts={
 en:['{n} queues ahead of you','You are next','It’s your turn — please come to the counter','Keep this page open. We will alert you when it is your turn.'],
 th:['มี {n} คิวก่อนหน้าคุณ','คุณเป็นคิวถัดไป','ถึงคิวของคุณแล้ว เชิญที่เคาน์เตอร์','เปิดหน้านี้ไว้ ระบบจะแจ้งเตือนเมื่อถึงคิวของคุณ'],
 ms:['{n} giliran sebelum anda','Anda yang seterusnya','Giliran anda — sila ke kaunter','Biarkan halaman ini terbuka. Kami akan memaklumkan apabila tiba giliran anda.'],
 zh:['您前面还有 {n} 位','下一位就是您','轮到您了，请到柜台办理','请保持此页面打开，轮到您时会提醒您。'],
 'zh-HK':['您前面還有 {n} 位','下一位就是您','輪到您了，請到櫃檯辦理','請保持此頁面開啟，輪到您時會提示您。'],
 hi:['आपसे पहले {n} लोग कतार में हैं','अगला नंबर आपका है','आपकी बारी आ गई है — कृपया काउंटर पर आएँ','यह पेज खुला रखें। आपकी बारी आने पर हम सूचित करेंगे।'],
 ru:['Перед вами в очереди: {n}','Вы следующий','Ваша очередь — подойдите к стойке','Не закрывайте эту страницу. Мы сообщим, когда подойдёт ваша очередь.'],
 ko:['앞에 {n}명이 대기 중입니다','다음 순서입니다','고객님 차례입니다. 카운터로 와 주세요','이 페이지를 열어 두세요. 차례가 되면 알려 드립니다.'],
 ja:['あなたの前に {n} 組お待ちです','次はあなたの番です','あなたの番です。カウンターへお越しください','このページを開いたままにしてください。順番が来たらお知らせします。'],
 de:['{n} vor Ihnen in der Warteschlange','Sie sind als Nächstes dran','Sie sind dran – bitte kommen Sie zum Schalter','Lassen Sie diese Seite geöffnet. Wir melden uns, wenn Sie an der Reihe sind.'],
 lo:['ມີ {n} ຄິວກ່ອນທ່ານ','ທ່ານແມ່ນຄິວຕໍ່ໄປ','ຮອດຄິວຂອງທ່ານແລ້ວ ເຊີນທີ່ເຄົາເຕີ','ເປີດໜ້ານີ້ໄວ້ ລະບົບຈະແຈ້ງເຕືອນເມື່ອຮອດຄິວຂອງທ່ານ'],
 fr:['{n} personne(s) avant vous','Vous êtes le prochain','C’est votre tour — rendez-vous au comptoir','Gardez cette page ouverte. Nous vous préviendrons quand ce sera votre tour.'],
 id:['{n} antrean sebelum Anda','Anda berikutnya','Giliran Anda — silakan ke konter','Biarkan halaman ini tetap terbuka. Kami akan memberi tahu saat giliran Anda tiba.'],
 my:['သင့်ရှေ့တွင် တန်းစီ {n} ခု ရှိသည်','နောက်တစ်ယောက်မှာ သင်ဖြစ်သည်','သင့်အလှည့် ရောက်ပါပြီ — ကောင်တာသို့ ကြွပါ','ဤစာမျက်နှာကို ဖွင့်ထားပါ။ သင့်အလှည့်ရောက်လျှင် အသိပေးပါမည်။'],
 vi:['Còn {n} lượt trước bạn','Bạn là lượt tiếp theo','Đến lượt bạn — mời đến quầy','Hãy giữ trang này mở. Chúng tôi sẽ báo khi đến lượt bạn.'],
 ar:['أمامك {n} في الطابور','أنت التالي','حان دورك — يرجى التوجه إلى الكاونتر','أبقِ هذه الصفحة مفتوحة، وسننبهك عندما يحين دورك.']
};
for(const [code,values] of Object.entries(queueTexts))Object.assign(window.KIOSK_TEXT[code],{queueAhead:values[0],queueNext:values[1],yourTurn:values[2],keepOpen:values[3]});

// Display-only currencies remain visible without offering an unavailable quote.
for(const [code,text] of Object.entries({"en": "Please contact staff", "th": "ติดต่อพนักงาน", "zh": "请联系工作人员", "zh-HK": "請聯絡職員", "ja": "スタッフにご相談ください", "ko": "직원에게 문의하세요", "ru": "Обратитесь к сотруднику", "hi": "कृपया कर्मचारी से संपर्क करें", "ar": "يرجى التواصل مع الموظف", "vi": "Vui lòng liên hệ nhân viên", "ms": "Sila hubungi kakitangan", "id": "Silakan hubungi staf", "de": "Bitte wenden Sie sich an das Personal", "fr": "Veuillez contacter le personnel", "lo": "ກະລຸນາຕິດຕໍ່ພະນັກງານ", "my": "ဝန်ထမ်းကို ဆက်သွယ်ပါ"}))window.KIOSK_TEXT[code].contactCounter=text;

// xchange: the branch control of a plain web visit (kiosk.html #branch-choice): label, search hint, empty choice.
const branchTexts={
 en:['Branch','Search branch','Choose a branch'],
 th:['สาขา','ค้นหาสาขา','เลือกสาขา'],
 ms:['Cawangan','Cari cawangan','Pilih cawangan'],
 zh:['分店','搜索分店','请选择分店'],
 'zh-HK':['分店','搜尋分店','請選擇分店'],
 hi:['शाखा','शाखा खोजें','शाखा चुनें'],
 ru:['Отделение','Поиск отделения','Выберите отделение'],
 ko:['지점','지점 검색','지점을 선택하세요'],
 ja:['店舗','店舗を検索','店舗を選択'],
 de:['Filiale','Filiale suchen','Filiale wählen'],
 lo:['ສາຂາ','ຄົ້ນຫາສາຂາ','ເລືອກສາຂາ'],
 fr:['Agence','Rechercher une agence','Choisir une agence'],
 id:['Cabang','Cari cabang','Pilih cabang'],
 my:['ဆိုင်ခွဲ','ဆိုင်ခွဲ ရှာရန်','ဆိုင်ခွဲ ရွေးပါ'],
 vi:['Chi nhánh','Tìm chi nhánh','Chọn chi nhánh'],
 ar:['الفرع','ابحث عن فرع','اختر الفرع']
};
for(const [code,values] of Object.entries(branchTexts))Object.assign(window.KIOSK_TEXT[code],{branchLabel:values[0],branchSearch:values[1],chooseBranch:values[2]});
