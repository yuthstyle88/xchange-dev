//! Free-text redaction (design `customer-app.md` P12): the one function that masks personal data in a
//! message before it is stored or logged. The assistant (APP-6) calls it before `assistant_messages`
//! is written, and `NotificationRequest::new` (`crate::notifications`) uses it to refuse PII in params.

const MASK: &str = "***";

/// Masks phone numbers, e-mail addresses, passport-like and national-id-like tokens, card-like digit
/// runs and OTP-like 4–8 digit codes. Text holding none of these is returned unchanged.
pub fn redact_text(text: &str) -> String {
    if text.is_empty() {
        return String::new();
    }

    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let n = chars.len();
    let mut spans: Vec<(usize, usize)> = Vec::new();

    // 1. Detect emails containing '@'
    for (idx, &(_, ch)) in chars.iter().enumerate() {
        if ch == '@' {
            let mut start_idx = idx;
            while start_idx > 0 {
                let prev_ch = chars[start_idx - 1].1;
                if prev_ch.is_ascii_alphanumeric()
                    || prev_ch == '.'
                    || prev_ch == '_'
                    || prev_ch == '%'
                    || prev_ch == '+'
                    || prev_ch == '-'
                {
                    start_idx -= 1;
                } else {
                    break;
                }
            }
            let mut end_idx = idx + 1;
            while end_idx < n {
                let next_ch = chars[end_idx].1;
                if next_ch.is_ascii_alphanumeric()
                    || next_ch == '.'
                    || next_ch == '-'
                    || next_ch == '_'
                {
                    end_idx += 1;
                } else {
                    break;
                }
            }
            while end_idx > idx + 1 && (chars[end_idx - 1].1 == '.' || chars[end_idx - 1].1 == '-')
            {
                end_idx -= 1;
            }
            if start_idx < idx && end_idx > idx + 1 {
                let byte_start = chars[start_idx].0;
                let byte_end = if end_idx < n {
                    chars[end_idx].0
                } else {
                    text.len()
                };
                spans.push((byte_start, byte_end));
            }
        }
    }

    // 2. Detect digit runs of 4 or more digits, including leading '+' or letters (like AA1234567)
    let mut i = 0;
    while i < n {
        if chars[i].1.is_ascii_digit() {
            let digit_start_idx = i;
            while i < n && chars[i].1.is_ascii_digit() {
                i += 1;
            }
            let digit_end_idx = i;
            let digit_count = digit_end_idx - digit_start_idx;
            if digit_count >= 4 {
                let mut span_start_idx = digit_start_idx;
                if digit_start_idx > 0 && chars[digit_start_idx - 1].1 == '+' {
                    span_start_idx = digit_start_idx - 1;
                } else if digit_start_idx > 0 && chars[digit_start_idx - 1].1.is_ascii_alphabetic()
                {
                    let mut letter_idx = digit_start_idx - 1;
                    while letter_idx > 0 && chars[letter_idx - 1].1.is_ascii_alphabetic() {
                        letter_idx -= 1;
                    }
                    span_start_idx = letter_idx;
                }
                let byte_start = chars[span_start_idx].0;
                let byte_end = if digit_end_idx < n {
                    chars[digit_end_idx].0
                } else {
                    text.len()
                };
                spans.push((byte_start, byte_end));
            }
        } else {
            i += 1;
        }
    }

    if spans.is_empty() {
        return text.to_string();
    }

    spans.sort_by_key(|&(start, _)| start);
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for (start, end) in spans {
        if let Some(last) = merged.last_mut() {
            if start <= last.1 {
                last.1 = last.1.max(end);
                continue;
            }
        }
        merged.push((start, end));
    }

    let mut result = String::with_capacity(text.len());
    let mut last_idx = 0;
    for (start, end) in merged {
        result.push_str(&text[last_idx..start]);
        result.push_str(MASK);
        last_idx = end;
    }
    result.push_str(&text[last_idx..]);
    result
}
