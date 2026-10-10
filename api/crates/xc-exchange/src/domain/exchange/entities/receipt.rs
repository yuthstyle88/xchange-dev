//! Receipt HTML generation.

use xc_kernel::money::{Money, Side};
use xc_kernel::time::bangkok;

use crate::domain::customer::value_objects::mode::Mode;
use crate::domain::exchange::value_objects::transaction_form::TransactionForm;

const TEMPLATE: &str = include_str!("receipt_template.html");

pub fn receipt_html(form: &TransactionForm, document_ref: &str, branch_name: &str) -> String {
    let title_val = escape_html(document_ref);
    let demo_banner_val = match form.customer.mode {
        Mode::Demo => {
            "<p style=\"font-size:8px;text-align:center;font-weight:bold;border:1px dashed;padding:3px\">SAMPLE / DEMO — NOT A VALID RECEIPT</p>"
        }
        Mode::Live => "",
    };
    let currency_val = escape_html(&form.currency);
    let rows_val = build_rows(form, document_ref, branch_name);
    let total_val = format_total(form.quote.principal);

    // Single-pass replacement over the template
    let mut result = String::with_capacity(TEMPLATE.len() + rows_val.len() + 256);
    let mut remaining = TEMPLATE;
    while let Some(start) = remaining.find("{{") {
        result.push_str(&remaining[..start]);
        let after_start = &remaining[start..];
        if let Some(end) = after_start.find("}}") {
            let placeholder = &after_start[..end + 2];
            match placeholder {
                "{{TITLE}}" => result.push_str(&title_val),
                "{{DEMO_BANNER}}" => result.push_str(demo_banner_val),
                "{{CURRENCY}}" => result.push_str(&currency_val),
                "{{ROWS}}" => result.push_str(&rows_val),
                "{{TOTAL}}" => result.push_str(&total_val),
                _ => result.push_str(placeholder),
            }
            remaining = &after_start[end + 2..];
        } else {
            result.push_str(after_start);
            remaining = "";
            break;
        }
    }
    result.push_str(remaining);
    result
}

fn build_rows(form: &TransactionForm, document_ref: &str, branch_name: &str) -> String {
    let side_str = match form.side {
        Side::Buy => "BUY",
        Side::Sell => "SELL",
    };

    let bkk_time = form.paid_at.with_timezone(&bangkok());
    let time_str = bkk_time.format("%d/%m/%Y %H:%M").to_string();
    let value_date_str = form.value_date.format("%Y-%m-%d").to_string();

    let booked_rate = format_booked_rate(form.client_rate);

    let rows: [(&str, &str, &str); 19] = [
        (" receipt-number", "Receipt No", document_ref),
        ("", "Type", side_str),
        (" passenger", "Customer", &form.customer.name),
        ("", "Currency", &form.currency),
        ("", "Amount", &form.amount.to_string()),
        ("", "Booked Rate", &booked_rate),
        ("", "Branch", branch_name),
        ("", "Document Ref", document_ref),
        ("", "Customer ID", &form.customer_id),
        ("", "THB Principal", &form.quote.principal.to_string()),
        ("", "Customer Fee", &form.fee.to_string()),
        ("", "THB Received", &form.quote.received.to_string()),
        ("", "THB Paid", &form.quote.paid.to_string()),
        ("", "Pay Via", form.method.as_str()),
        ("", "Payment Ref", &form.payment_ref),
        ("", "Booking No", &form.booking),
        ("", "Time", &time_str),
        ("", "Value Date", &value_date_str),
        ("", "Staff", &form.staff),
    ];

    let mut out = String::with_capacity(2048);
    for (class_suffix, label, value) in rows {
        out.push_str("<div class=\"row");
        out.push_str(class_suffix);
        out.push_str("\"><dt>");
        out.push_str(label);
        out.push_str("</dt><dd>");
        out.push_str(&escape_html(value));
        out.push_str("</dd></div>");
    }
    out
}

fn format_total(principal: Money) -> String {
    let s = principal.to_string();
    let (whole, frac) = s.split_once('.').unwrap_or((&s, "00"));
    let bytes = whole.as_bytes();
    let len = bytes.len();
    let mut result = String::with_capacity(len + len / 3 + 4);
    for (i, &b) in bytes.iter().enumerate() {
        if i > 0 && (len - i) % 3 == 0 {
            result.push(',');
        }
        result.push(b as char);
    }
    result.push('.');
    result.push_str(frac);
    result
}

fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            other => out.push(other),
        }
    }
    out
}

fn format_booked_rate(d: rust_decimal::Decimal) -> String {
    let norm = d.normalize();
    let scale = norm.scale();
    if scale < 4 {
        let mut rescaled = norm;
        rescaled.rescale(4);
        rescaled.to_string()
    } else {
        norm.to_string()
    }
}
