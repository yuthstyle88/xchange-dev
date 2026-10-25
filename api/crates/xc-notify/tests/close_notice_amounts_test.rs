//! Ticket #5113: close-count notices show counted and expected THB when present.

mod common;

use std::collections::BTreeMap;

use common::*;

fn params_with(c: Option<&str>, e: Option<&str>) -> BTreeMap<String, String> {
    let mut map = params(&[("branch", "Siam"), ("shift", "S-1"), ("by", "Somsee")]);
    if let Some(c) = c {
        map.insert("counted_thb".into(), c.into());
    }
    if let Some(e) = e {
        map.insert("expected_thb".into(), e.into());
    }
    map
}

#[test]
fn n1_waiting_th_both_present() {
    let registry = TemplateRegistry::generated();
    let template = registry.get("close_count_waiting").expect("template");
    let p = params_with(Some("2200.00"), Some("2500.00"));
    let r = template.render(Locale::Th, &p);
    assert_eq!(
        r.body,
        "มีการส่งยอดนับเงินปิดกะ S-1 ที่สาขา Siam โดย Somsee รอการรับรอง (นับได้ 2200.00 บาท, ควรมี 2500.00 บาท)"
    );
    assert!(
        r.body
            .ends_with("รอการรับรอง (นับได้ 2200.00 บาท, ควรมี 2500.00 บาท)")
    );
}

#[test]
fn n2_waiting_en_both_present() {
    let registry = TemplateRegistry::generated();
    let template = registry.get("close_count_waiting").expect("template");
    let p = params_with(Some("2200.00"), Some("2500.00"));
    let r = template.render(Locale::En, &p);
    assert_eq!(
        r.body,
        "Close count for shift S-1 at branch Siam submitted by Somsee is waiting for certification. Counted THB 2200.00, expected THB 2500.00."
    );
    assert!(
        r.body
            .ends_with("certification. Counted THB 2200.00, expected THB 2500.00.")
    );
}

#[test]
fn n3_waiting_zh_both_present() {
    let registry = TemplateRegistry::generated();
    let template = registry.get("close_count_waiting").expect("template");
    let p = params_with(Some("2200.00"), Some("2500.00"));
    let r = template.render(Locale::Zh, &p);
    assert_eq!(
        r.body,
        "Siam 分行班次 S-1 的交班清点（由 Somsee 提交）正在等待确认。清点 2200.00 泰铢，应有 2500.00 泰铢。"
    );
    assert!(
        r.body
            .ends_with("正在等待确认。清点 2200.00 泰铢，应有 2500.00 泰铢。")
    );
}

#[test]
fn n4_certified_all_locales_both_present() {
    let registry = TemplateRegistry::generated();
    let template = registry.get("shift_close_certified").expect("template");
    let p = params_with(Some("2200.00"), Some("2500.00"));

    let th = template.render(Locale::Th, &p);
    assert_eq!(
        th.body,
        "การนับเงินปิดกะ S-1 ที่สาขา Siam ได้รับการรับรองโดย Somsee (นับได้ 2200.00 บาท, ควรมี 2500.00 บาท)"
    );
    assert!(
        th.body
            .ends_with("Somsee (นับได้ 2200.00 บาท, ควรมี 2500.00 บาท)")
    );

    let en = template.render(Locale::En, &p);
    assert_eq!(
        en.body,
        "Shift close for shift S-1 at branch Siam was certified by Somsee. Counted THB 2200.00, expected THB 2500.00."
    );
    assert!(
        en.body
            .ends_with("certified by Somsee. Counted THB 2200.00, expected THB 2500.00.")
    );

    let zh = template.render(Locale::Zh, &p);
    assert_eq!(
        zh.body,
        "Siam 分行班次 S-1 的交班清点已由 Somsee 确认。清点 2200.00 泰铢，应有 2500.00 泰铢。"
    );
    assert!(
        zh.body
            .ends_with("确认。清点 2200.00 泰铢，应有 2500.00 泰铢。")
    );
}

#[test]
fn n5_either_all_locales_only_counted_present() {
    let registry = TemplateRegistry::generated();
    let waiting = registry.get("close_count_waiting").expect("template");
    let certified = registry.get("shift_close_certified").expect("template");
    let p = params_with(Some("2200.00"), None);

    assert_eq!(
        waiting.render(Locale::Th, &p).body,
        "มีการส่งยอดนับเงินปิดกะ S-1 ที่สาขา Siam โดย Somsee รอการรับรอง (นับได้ 2200.00 บาท)"
    );
    assert_eq!(
        waiting.render(Locale::En, &p).body,
        "Close count for shift S-1 at branch Siam submitted by Somsee is waiting for certification. Counted THB 2200.00."
    );
    assert_eq!(
        waiting.render(Locale::Zh, &p).body,
        "Siam 分行班次 S-1 的交班清点（由 Somsee 提交）正在等待确认。清点 2200.00 泰铢。"
    );

    assert_eq!(
        certified.render(Locale::Th, &p).body,
        "การนับเงินปิดกะ S-1 ที่สาขา Siam ได้รับการรับรองโดย Somsee (นับได้ 2200.00 บาท)"
    );
    assert_eq!(
        certified.render(Locale::En, &p).body,
        "Shift close for shift S-1 at branch Siam was certified by Somsee. Counted THB 2200.00."
    );
    assert_eq!(
        certified.render(Locale::Zh, &p).body,
        "Siam 分行班次 S-1 的交班清点已由 Somsee 确认。清点 2200.00 泰铢。"
    );
}

#[test]
fn n6_either_all_locales_neither_present() {
    let registry = TemplateRegistry::generated();
    let waiting = registry.get("close_count_waiting").expect("template");
    let certified = registry.get("shift_close_certified").expect("template");
    let p = params_with(None, None);

    assert_eq!(
        waiting.render(Locale::Th, &p).body,
        "มีการส่งยอดนับเงินปิดกะ S-1 ที่สาขา Siam โดย Somsee รอการรับรอง"
    );
    assert_eq!(
        waiting.render(Locale::En, &p).body,
        "Close count for shift S-1 at branch Siam submitted by Somsee is waiting for certification."
    );
    assert_eq!(
        waiting.render(Locale::Zh, &p).body,
        "Siam 分行班次 S-1 的交班清点（由 Somsee 提交）正在等待确认。"
    );

    assert_eq!(
        certified.render(Locale::Th, &p).body,
        "การนับเงินปิดกะ S-1 ที่สาขา Siam ได้รับการรับรองโดย Somsee"
    );
    assert_eq!(
        certified.render(Locale::En, &p).body,
        "Shift close for shift S-1 at branch Siam was certified by Somsee."
    );
    assert_eq!(
        certified.render(Locale::Zh, &p).body,
        "Siam 分行班次 S-1 的交班清点已由 Somsee 确认。"
    );
}

#[test]
fn n7_either_all_locales_only_expected_present() {
    let registry = TemplateRegistry::generated();
    let waiting = registry.get("close_count_waiting").expect("template");
    let certified = registry.get("shift_close_certified").expect("template");
    let p = params_with(None, Some("2500.00"));

    assert_eq!(
        waiting.render(Locale::Th, &p).body,
        "มีการส่งยอดนับเงินปิดกะ S-1 ที่สาขา Siam โดย Somsee รอการรับรอง"
    );
    assert_eq!(
        waiting.render(Locale::En, &p).body,
        "Close count for shift S-1 at branch Siam submitted by Somsee is waiting for certification."
    );
    assert_eq!(
        waiting.render(Locale::Zh, &p).body,
        "Siam 分行班次 S-1 的交班清点（由 Somsee 提交）正在等待确认。"
    );

    assert_eq!(
        certified.render(Locale::Th, &p).body,
        "การนับเงินปิดกะ S-1 ที่สาขา Siam ได้รับการรับรองโดย Somsee"
    );
    assert_eq!(
        certified.render(Locale::En, &p).body,
        "Shift close for shift S-1 at branch Siam was certified by Somsee."
    );
    assert_eq!(
        certified.render(Locale::Zh, &p).body,
        "Siam 分行班次 S-1 的交班清点已由 Somsee 确认。"
    );
}

#[test]
fn n8_both_version_2_and_titles_unchanged() {
    let registry = TemplateRegistry::generated();
    let waiting = registry.get("close_count_waiting").expect("template");
    let certified = registry.get("shift_close_certified").expect("template");

    assert_eq!(waiting.version(), 2);
    assert_eq!(certified.version(), 2);

    let p = params_with(Some("2200.00"), Some("2500.00"));

    assert_eq!(
        waiting.render(Locale::Th, &p).title,
        "การนับเงินปิดกะรอการรับรอง"
    );
    assert_eq!(
        waiting.render(Locale::En, &p).title,
        "Close count waiting for certification"
    );
    assert_eq!(waiting.render(Locale::Zh, &p).title, "交班清点待确认");

    assert_eq!(
        certified.render(Locale::Th, &p).title,
        "รับรองการปิดกะเรียบร้อยแล้ว"
    );
    assert_eq!(
        certified.render(Locale::En, &p).title,
        "Shift close certified"
    );
    assert_eq!(certified.render(Locale::Zh, &p).title, "交班清点已确认");
}
