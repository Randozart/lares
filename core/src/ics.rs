//! Minimal ICS (iCalendar) parser for birthday/occasion import.
//!
//! Handles the narrow slice Google Calendar exports for birthday and
//! anniversary calendars: `VEVENT` blocks with `SUMMARY`, `DTSTART` (all-day
//! `VALUE=DATE` preferred, datetime tolerated), and `RRULE:FREQ=YEARLY`.
//! Pure and unit-tested against a realistic export fixture; no network.

/// One parsed calendar event, normalized.
#[derive(Debug, PartialEq)]
pub struct IcsEvent {
    /// Event title, e.g. "Emma's Birthday".
    pub summary: String,
    /// Event date: "MM-DD" for yearly events, "YYYY-MM-DD" otherwise.
    pub date: String,
    /// Whether the event recurs yearly.
    pub yearly: bool,
}

/// Whether a raw line continues the previous (folded) line.
fn is_continuation(line: &str) -> bool {
    line.starts_with(' ') || line.starts_with('\t')
}

/// Strip exactly one folding whitespace character from a continuation line.
fn strip_fold_marker(line: &str) -> &str {
    line.strip_prefix(' ')
        .or_else(|| line.strip_prefix('\t'))
        .unwrap_or(line)
}

/// Unfold RFC 5545 folded lines (CRLF or LF followed by space/tab).
///
/// Removes the line break plus exactly one folding whitespace character,
/// per RFC 5545 §3.1. Single pass: continuation lines are appended to the
/// merged buffer without a separating newline.
fn unfold(text: &str) -> String {
    let mut merged = String::with_capacity(text.len());
    for line in text.lines() {
        if is_continuation(line) {
            merged.push_str(strip_fold_marker(line));
        } else {
            if !merged.is_empty() {
                merged.push('\n');
            }
            merged.push_str(line);
        }
    }
    merged.push('\n');
    merged
}

/// Extract the bare value from a `NAME;PARAM=...:VALUE` content line.
fn line_value(line: &str) -> Option<&str> {
    line.split_once(':').map(|(_, value)| value.trim())
}

/// Normalize an 8-digit date (`YYYYMMDD`) or datetime value to `YYYY-MM-DD`.
fn normalize_date(value: &str) -> Option<String> {
    let digits: String = value.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.len() < 8 {
        return None;
    }
    let (y, m, d) = (
        &digits[0..4],
        &digits[4..6],
        &digits[6..8],
    );
    Some(format!("{y}-{m}-{d}"))
}

/// Accumulates the fields of one VEVENT block.
#[derive(Default)]
struct EventBuilder {
    summary: String,
    dtstart: String,
    yearly: bool,
}

impl EventBuilder {
    /// Absorb one content line into the builder state.
    ///
    /// Delegates to per-field handlers; each line matches at most one field.
    fn absorb(&mut self, line: &str) {
        self.absorb_summary(line);
        self.absorb_dtstart(line);
        self.absorb_rrule(line);
    }

    /// Capture a `SUMMARY` line.
    fn absorb_summary(&mut self, line: &str) {
        if !line.starts_with("SUMMARY") {
            return;
        }
        if let Some(value) = line_value(line) {
            self.summary = unescape_text(value);
        }
    }

    /// Capture a `DTSTART` line as a normalized date.
    fn absorb_dtstart(&mut self, line: &str) {
        if !line.starts_with("DTSTART") {
            return;
        }
        if let Some(value) = line_value(line) {
            self.dtstart = normalize_date(value).unwrap_or_default();
        }
    }

    /// Detect yearly recurrence from an `RRULE` line.
    fn absorb_rrule(&mut self, line: &str) {
        if !line.starts_with("RRULE") {
            return;
        }
        if let Some(value) = line_value(line) {
            self.yearly = value.to_uppercase().contains("FREQ=YEARLY");
        }
    }

    /// Convert the accumulated fields into a normalized [`IcsEvent`], if valid.
    fn finish(self, events: &mut Vec<IcsEvent>) {
        if self.summary.is_empty() || self.dtstart.len() < 10 {
            return;
        }
        let is_yearly = self.yearly || is_recurring_title(&self.summary);
        let date = if is_yearly {
            self.dtstart[5..].to_string()
        } else {
            self.dtstart
        };
        events.push(IcsEvent {
            summary: self.summary,
            date,
            yearly: is_yearly,
        });
    }
}

/// Parse all events from an ICS document body.
pub fn parse_ics(text: &str) -> Vec<IcsEvent> {
    let unfolded = unfold(text);
    let mut events = Vec::new();
    let mut builder = EventBuilder::default();
    let mut in_event = false;
    for line in unfolded.lines() {
        match line.trim() {
            "BEGIN:VEVENT" => {
                in_event = true;
                builder = EventBuilder::default();
            }
            "END:VEVENT" => {
                if in_event {
                    std::mem::take(&mut builder).finish(&mut events);
                }
                in_event = false;
            }
            trimmed => {
                if in_event {
                    builder.absorb(trimmed);
                }
            }
        }
    }
    events
}

/// Whether the title itself implies a yearly recurrence ("birthday", ...).
fn is_recurring_title(summary: &str) -> bool {
    let lower = summary.to_lowercase();
    lower.contains("birthday") || lower.contains("anniversary")
}

/// Unescape RFC 5545 text escapes (`\\n`, `\\,`, `\\;`, `\\\\`).
fn unescape_text(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') | Some('N') => out.push(' '),
                Some(escaped) => out.push(escaped),
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Derive a person's name from an event title.
///
/// "Emma's Birthday" → "Emma"; "Birthday of Emma" → "Emma"; unmatched titles
/// return `None` (occasion without a person).
pub fn person_name_from_title(title: &str) -> Option<String> {
    let lower = title.to_lowercase();
    for keyword in ["birthday", "anniversary"] {
        if let Some(idx) = lower.find(keyword) {
            let (before, after) = title.split_at(idx);
            let after = &after[keyword.len()..];
            let from_before = clean_name_fragment(before);
            if let Some(name) = from_before {
                return Some(name);
            }
            return clean_name_fragment(after);
        }
    }
    None
}

/// Strip one leading filler prefix from a fragment; returns `None` untouched.
fn strip_one_prefix(cleaned: &str) -> Option<&str> {
    let lower = cleaned.to_lowercase();
    ["of ", "the ", "& ", ", ", "- "]
        .into_iter()
        .find(|prefix| lower.starts_with(*prefix))
        .map(|prefix| cleaned[prefix.len()..].trim_start())
}

/// Trim possessives and filler words from a name fragment.
fn clean_name_fragment(fragment: &str) -> Option<String> {
    let mut cleaned = fragment
        .trim()
        .trim_end_matches([',', '-', ':'])
        .trim();
    for suffix in ["'s", "’s"] {
        if let Some(stripped) = cleaned.strip_suffix(suffix) {
            cleaned = stripped.trim_end();
        }
    }
    // Strip leading filler ("of Emma" → "Emma").
    while let Some(remaining) = strip_one_prefix(cleaned) {
        cleaned = remaining;
    }
    let lower = cleaned.to_lowercase();
    if cleaned.is_empty()
        || ["of", "the", "wedding", "our", "my", "family"].contains(&lower.as_str())
    {
        return None;
    }
    Some(cleaned.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A realistic Google Calendar birthday-export slice.
    const FIXTURE: &str = "BEGIN:VCALENDAR\r\n\
VERSION:2.0\r\n\
PRODID:-//Google Inc//Google Calendar 70.9054//EN\r\n\
BEGIN:VEVENT\r\n\
DTSTART;VALUE=DATE:19900315\r\n\
DTEND;VALUE=DATE:19900316\r\n\
RRULE:FREQ=YEARLY\r\n\
DTSTAMP:20260901T000000Z\r\n\
UID:abc123\r\n\
SUMMARY:Emma's Birthday\r\n\
END:VEVENT\r\n\
BEGIN:VEVENT\r\n\
DTSTART;VALUE=DATE:19880707\r\n\
DTEND;VALUE=DATE:19880708\r\n\
RRULE:FREQ=YEARLY\r\n\
UID:def456\r\n\
SUMMARY:Wedding Anniversary\r\n\
END:VEVENT\r\n\
BEGIN:VEVENT\r\n\
DTSTART:20261005T190000Z\r\n\
DTEND:20261005T210000Z\r\n\
UID:ghi789\r\n\
SUMMARY:Dinner with the Smiths\\, bring wine\r\n\
END:VEVENT\r\n\
BEGIN:VEVENT\r\n\
DTSTART;VALUE=DATE:20261224\r\n\
UID:jkl012\r\n\
SUMMARY:Family photo shoot (folded\r\n  continuation line)\r\n\
END:VEVENT\r\n\
END:VCALENDAR\r\n";

    #[test]
    fn parses_yearly_birthdays_to_mmdd() {
        let events = parse_ics(FIXTURE);
        assert_eq!(events.len(), 4);
        assert_eq!(events[0].summary, "Emma's Birthday");
        assert_eq!(events[0].date, "03-15");
        assert!(events[0].yearly);
    }

    #[test]
    fn anniversary_without_year_still_yearly() {
        let events = parse_ics(FIXTURE);
        assert_eq!(events[1].summary, "Wedding Anniversary");
        assert_eq!(events[1].date, "07-07");
        assert!(events[1].yearly);
    }

    #[test]
    fn one_time_event_keeps_full_date_and_unescapes() {
        let events = parse_ics(FIXTURE);
        assert_eq!(events[2].date, "2026-10-05");
        assert!(!events[2].yearly);
        assert_eq!(events[2].summary, "Dinner with the Smiths, bring wine");
    }

    #[test]
    fn folded_lines_are_unfolded() {
        let events = parse_ics(FIXTURE);
        assert_eq!(events[3].summary, "Family photo shoot (folded continuation line)");
        assert_eq!(events[3].date, "2026-12-24");
    }

    #[test]
    fn person_names_extracted_from_titles() {
        assert_eq!(person_name_from_title("Emma's Birthday").as_deref(), Some("Emma"));
        assert_eq!(person_name_from_title("Birthday of Emma").as_deref(), Some("Emma"));
        assert_eq!(person_name_from_title("John & Mary Anniversary").as_deref(), Some("John & Mary"));
        assert!(person_name_from_title("Wedding Anniversary").is_none());
    }

    #[test]
    fn datetime_dtstart_normalized() {
        let events = parse_ics(FIXTURE);
        assert_eq!(normalize_date("20261005T190000Z").as_deref(), Some("2026-10-05"));
        assert_eq!(events[2].date, "2026-10-05");
    }
}
