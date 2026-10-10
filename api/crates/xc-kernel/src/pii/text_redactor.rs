//! Free-text redaction (design `customer-app.md` P12): the one function that masks personal data in a
//! message before it is stored or logged. The assistant (APP-6) calls it before `assistant_messages`
//! is written, and `NotificationRequest::new` (`crate::notifications`) uses it to refuse PII in params.

const MASK: &str = "***";

fn is_digit(c: char) -> bool {
    matches!(c, '0'..='9' | '๐'..='๙' | '０'..='９')
}

fn is_local_part_char(c: char) -> bool {
    c.is_alphabetic()
        || ('\u{0e01}'..='\u{0e4e}').contains(&c)
        || is_digit(c)
        || c == '.'
        || c == '_'
        || c == '%'
        || c == '+'
        || c == '-'
}

fn is_domain_char(c: char) -> bool {
    c.is_alphanumeric()
        || ('\u{0e01}'..='\u{0e4e}').contains(&c)
        || c == '.'
        || c == '-'
        || c == '_'
}

struct DigitRun {
    start: usize,
    end: usize,
    count: usize,
}

/// Masks phone numbers, e-mail addresses, passport-like and national-id-like tokens, card-like digit
/// runs and OTP-like 4–8 digit codes. Text holding none of these is returned unchanged.
pub fn redact_text(text: &str) -> String {
    if text.is_empty() {
        return String::new();
    }

    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let n = chars.len();
    let mut spans: Vec<(usize, usize)> = Vec::new();

    // 1. Detect emails containing '@' (with optional single space on each side)
    for (idx, &(_, ch)) in chars.iter().enumerate() {
        if ch == '@' {
            let local_end = if idx > 0 && chars[idx - 1].1 == ' ' {
                if idx >= 2 && chars[idx - 2].1 == ' ' {
                    continue;
                }
                idx - 1
            } else {
                idx
            };

            let mut local_start = local_end;
            while local_start > 0 && is_local_part_char(chars[local_start - 1].1) {
                local_start -= 1;
            }

            if local_start == local_end {
                continue;
            }

            let domain_start = if idx + 1 < n && chars[idx + 1].1 == ' ' {
                if idx + 2 < n && chars[idx + 2].1 == ' ' {
                    continue;
                }
                idx + 2
            } else {
                idx + 1
            };

            if domain_start >= n {
                continue;
            }

            let mut domain_end = domain_start;
            while domain_end < n && is_domain_char(chars[domain_end].1) {
                domain_end += 1;
            }

            while domain_end > domain_start
                && (chars[domain_end - 1].1 == '.'
                    || chars[domain_end - 1].1 == '-'
                    || chars[domain_end - 1].1 == '_')
            {
                domain_end -= 1;
            }

            if domain_end <= domain_start {
                continue;
            }

            let domain_slice = &chars[domain_start..domain_end];
            if let Some(last_dot_rel_idx) = domain_slice.iter().rposition(|&(_, c)| c == '.') {
                if last_dot_rel_idx > 0 {
                    let tld = &domain_slice[last_dot_rel_idx + 1..];
                    if tld.len() >= 2
                        && tld.iter().all(|&(_, c)| {
                            c.is_alphabetic() || ('\u{0e01}'..='\u{0e4e}').contains(&c)
                        })
                    {
                        let byte_start = chars[local_start].0;
                        let byte_end = if domain_end < n {
                            chars[domain_end].0
                        } else {
                            text.len()
                        };
                        spans.push((byte_start, byte_end));
                    }
                }
            }
        }
    }

    // 2. Detect digit groups: digit runs joined by a single '-' or a single space
    let mut runs: Vec<DigitRun> = Vec::new();
    let mut i = 0;
    while i < n {
        if is_digit(chars[i].1) {
            let start = i;
            while i < n && is_digit(chars[i].1) {
                i += 1;
            }
            runs.push(DigitRun {
                start,
                end: i,
                count: i - start,
            });
        } else {
            i += 1;
        }
    }

    let mut run_groups: Vec<Vec<DigitRun>> = Vec::new();
    for run in runs {
        if let Some(current_group) = run_groups.last_mut() {
            let last_run = current_group.last().unwrap();
            if run.start == last_run.end + 1 {
                let joiner = chars[last_run.end].1;
                if joiner == '-' || joiner == ' ' {
                    current_group.push(run);
                    continue;
                }
            }
        }
        run_groups.push(vec![run]);
    }

    for group in run_groups {
        let total_digits: usize = group.iter().map(|r| r.count).sum();
        let max_run_digits: usize = group.iter().map(|r| r.count).max().unwrap_or(0);
        if max_run_digits >= 4 || total_digits >= 6 {
            let first_run = group.first().unwrap();
            let last_run = group.last().unwrap();
            let mut span_start_idx = first_run.start;
            let span_end_idx = last_run.end;

            if span_start_idx > 0 && chars[span_start_idx - 1].1 == '+' {
                span_start_idx -= 1;
            } else if span_start_idx > 0 && chars[span_start_idx - 1].1.is_ascii_alphabetic() {
                let mut letter_idx = span_start_idx - 1;
                while letter_idx > 0 && chars[letter_idx - 1].1.is_ascii_alphabetic() {
                    letter_idx -= 1;
                }
                span_start_idx = letter_idx;
            }

            let byte_start = chars[span_start_idx].0;
            let byte_end = if span_end_idx < n {
                chars[span_end_idx].0
            } else {
                text.len()
            };
            spans.push((byte_start, byte_end));
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
