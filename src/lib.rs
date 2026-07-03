//! srt-tools: a small, dependency-light library for parsing and manipulating
//! SubRip (`.srt`) and WebVTT (`.vtt`) subtitle files.
//!
//! The library is built around two types:
//!   * [`Timestamp`] — a millisecond-precision point in time with exact,
//!     lossless SRT (`HH:MM:SS,mmm`) and VTT (`HH:MM:SS.mmm`) formatting.
//!   * [`Cue`] — a single subtitle entry (index, start, end, text).
//!
//! Parsing is hand-written (no regex / no heavy subtitle crate) and is tolerant
//! of the common real-world quirks: CRLF or LF line endings, a BOM, blank lines
//! between cues, VTT headers / `NOTE` blocks / cue identifiers, and `.`-vs-`,`
//! millisecond separators.

use std::fmt;
use thiserror::Error;

/// Errors produced while parsing subtitle input.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ParseError {
    #[error("invalid timestamp {0:?}: expected HH:MM:SS,mmm or HH:MM:SS.mmm")]
    BadTimestamp(String),
    #[error("cue starting near line {line}: missing '-->' timing line")]
    MissingArrow { line: usize },
    #[error("cue starting near line {line}: malformed timing line {text:?}")]
    BadTiming { line: usize, text: String },
    #[error("input contained no subtitle cues")]
    Empty,
}

/// Subtitle container format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Srt,
    Vtt,
}

impl Format {
    /// Infer a format from a file path's extension (case-insensitive).
    /// Anything that is not `.vtt` is treated as SRT.
    pub fn from_path(path: &str) -> Format {
        let lower = path.to_ascii_lowercase();
        if lower.ends_with(".vtt") {
            Format::Vtt
        } else {
            Format::Srt
        }
    }
}

/// A millisecond-precision timestamp.
///
/// Stored as a single `i64` count of milliseconds so that arithmetic (shift,
/// scale) is exact and a shift that would move a cue before zero can be
/// represented transiently before being clamped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Timestamp {
    pub ms: i64,
}

impl Timestamp {
    pub fn from_ms(ms: i64) -> Self {
        Timestamp { ms }
    }

    pub fn from_hmsm(h: i64, m: i64, s: i64, ms: i64) -> Self {
        Timestamp {
            ms: ((h * 60 + m) * 60 + s) * 1000 + ms,
        }
    }

    /// Saturating shift: never goes below zero (a subtitle cannot start before
    /// the start of the media).
    pub fn shifted(self, delta_ms: i64) -> Self {
        Timestamp {
            ms: (self.ms + delta_ms).max(0),
        }
    }

    /// Linear scale about the origin, rounded to the nearest millisecond.
    pub fn scaled(self, factor: f64) -> Self {
        let scaled = (self.ms as f64) * factor;
        Timestamp {
            ms: scaled.round() as i64,
        }
    }

    /// Parse `HH:MM:SS,mmm` or `HH:MM:SS.mmm`. Hours may be 1+ digits; a 2-digit
    /// `MM:SS.mmm` form (no hours, as VTT permits) is also accepted. The
    /// millisecond field is optional and may be 1-3 digits.
    pub fn parse(raw: &str) -> Result<Self, ParseError> {
        let s = raw.trim();
        // Split off milliseconds on either ',' or '.'.
        let (clock, millis) = match s.find([',', '.']) {
            Some(idx) => {
                let (c, rest) = s.split_at(idx);
                (c, &rest[1..])
            }
            None => (s, ""),
        };

        let ms: i64 = if millis.is_empty() {
            0
        } else {
            if millis.len() > 3 || !millis.bytes().all(|b| b.is_ascii_digit()) {
                return Err(ParseError::BadTimestamp(raw.to_string()));
            }
            // Right-pad so "5" => 500ms, "50" => 500... no: "5" => 500, "05" => 50.
            let mut padded = millis.to_string();
            while padded.len() < 3 {
                padded.push('0');
            }
            padded
                .parse()
                .map_err(|_| ParseError::BadTimestamp(raw.to_string()))?
        };

        let parts: Vec<&str> = clock.split(':').collect();
        let (h, m, sec) = match parts.as_slice() {
            [h, m, s] => (*h, *m, *s),
            [m, s] => ("0", *m, *s),
            _ => return Err(ParseError::BadTimestamp(raw.to_string())),
        };

        let h: i64 = parse_field(h, raw)?;
        let m: i64 = parse_field(m, raw)?;
        let sec: i64 = parse_field(sec, raw)?;

        if !(0..60).contains(&m) || !(0..60).contains(&sec) {
            return Err(ParseError::BadTimestamp(raw.to_string()));
        }

        Ok(Timestamp::from_hmsm(h, m, sec, ms))
    }

    fn components(self) -> (i64, i64, i64, i64) {
        let ms = self.ms.max(0);
        let h = ms / 3_600_000;
        let m = (ms % 3_600_000) / 60_000;
        let s = (ms % 60_000) / 1000;
        let milli = ms % 1000;
        (h, m, s, milli)
    }

    /// Format as SRT (`HH:MM:SS,mmm`).
    pub fn to_srt(self) -> String {
        let (h, m, s, ms) = self.components();
        format!("{:02}:{:02}:{:02},{:03}", h, m, s, ms)
    }

    /// Format as VTT (`HH:MM:SS.mmm`).
    pub fn to_vtt(self) -> String {
        let (h, m, s, ms) = self.components();
        format!("{:02}:{:02}:{:02}.{:03}", h, m, s, ms)
    }
}

fn parse_field(field: &str, raw: &str) -> Result<i64, ParseError> {
    let f = field.trim();
    if f.is_empty() || !f.bytes().all(|b| b.is_ascii_digit()) {
        return Err(ParseError::BadTimestamp(raw.to_string()));
    }
    f.parse()
        .map_err(|_| ParseError::BadTimestamp(raw.to_string()))
}

impl fmt::Display for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_srt())
    }
}

/// A single subtitle cue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cue {
    /// 1-based sequence number. Not authoritative — [`fix`] / [`renumber`]
    /// rewrites it; parsing keeps whatever was present (or assigns by order).
    pub index: usize,
    pub start: Timestamp,
    pub end: Timestamp,
    /// The cue body, with internal `\n` between lines and no trailing newline.
    pub text: String,
}

impl Cue {
    pub fn new(index: usize, start: Timestamp, end: Timestamp, text: impl Into<String>) -> Self {
        Cue {
            index,
            start,
            end,
            text: text.into(),
        }
    }

    pub fn duration_ms(&self) -> i64 {
        self.end.ms - self.start.ms
    }

    /// A cue is "empty" if its body has no non-whitespace characters.
    pub fn is_empty(&self) -> bool {
        self.text.trim().is_empty()
    }
}

/// Normalize line endings to `\n` and strip a leading UTF-8 BOM.
fn normalize(input: &str) -> String {
    let stripped = input.strip_prefix('\u{feff}').unwrap_or(input);
    stripped.replace("\r\n", "\n").replace('\r', "\n")
}

/// Split a timing line on `-->`, trimming any trailing VTT cue settings
/// (e.g. `position:50%`) from the end timestamp token.
fn split_timing(line: &str) -> Option<(&str, &str)> {
    let (lhs, rhs) = line.split_once("-->")?;
    let start = lhs.trim();
    // The end side may carry cue settings after whitespace: take the first token.
    let end = rhs.trim().split_whitespace().next().unwrap_or("");
    Some((start, end))
}

fn is_timing_line(line: &str) -> bool {
    line.contains("-->")
}

/// Parse subtitle text (SRT or VTT — the parser auto-detects per cue) into a
/// vector of [`Cue`]s. VTT headers, `NOTE` blocks, `STYLE` blocks and cue
/// identifiers are skipped. Indices are assigned by appearance order so the
/// result is always sequentially numbered from 1.
pub fn parse(input: &str) -> Result<Vec<Cue>, ParseError> {
    let text = normalize(input);
    let lines: Vec<&str> = text.lines().collect();
    let mut cues = Vec::new();
    let mut i = 0;
    let n = lines.len();
    let mut next_index = 1;

    while i < n {
        let line = lines[i];

        // Skip blank lines.
        if line.trim().is_empty() {
            i += 1;
            continue;
        }

        // Skip the WEBVTT signature line (possibly with trailing description).
        if line.trim_start().starts_with("WEBVTT") {
            i += 1;
            continue;
        }

        // Skip NOTE / STYLE / REGION blocks (VTT): consume until a blank line.
        let head = line.trim_start();
        if head == "NOTE" || head.starts_with("NOTE ") || head == "STYLE" || head == "REGION" {
            i += 1;
            while i < n && !lines[i].trim().is_empty() {
                i += 1;
            }
            continue;
        }

        // This non-blank line begins a cue. It may be:
        //   * a numeric SRT index line (then the next line is the timing), or
        //   * a VTT cue identifier (then the next line is the timing), or
        //   * the timing line itself.
        let cue_start_line = i + 1; // 1-based for error messages

        let timing_line_idx = if is_timing_line(line) {
            i
        } else {
            // Treat `line` as an identifier/index; timing must be next.
            if i + 1 < n && is_timing_line(lines[i + 1]) {
                i + 1
            } else {
                return Err(ParseError::MissingArrow {
                    line: cue_start_line,
                });
            }
        };

        let timing = lines[timing_line_idx];
        let (start_s, end_s) = split_timing(timing).ok_or_else(|| ParseError::BadTiming {
            line: cue_start_line,
            text: timing.to_string(),
        })?;
        let start = Timestamp::parse(start_s).map_err(|_| ParseError::BadTiming {
            line: cue_start_line,
            text: timing.to_string(),
        })?;
        let end = Timestamp::parse(end_s).map_err(|_| ParseError::BadTiming {
            line: cue_start_line,
            text: timing.to_string(),
        })?;

        // Collect the body: every line after the timing line up to a blank line.
        let mut body: Vec<&str> = Vec::new();
        let mut j = timing_line_idx + 1;
        while j < n && !lines[j].trim().is_empty() {
            body.push(lines[j]);
            j += 1;
        }

        cues.push(Cue {
            index: next_index,
            start,
            end,
            text: body.join("\n"),
        });
        next_index += 1;

        i = j;
    }

    if cues.is_empty() {
        return Err(ParseError::Empty);
    }
    Ok(cues)
}

/// Serialize cues to SRT text. Uses `\n` line endings and a trailing newline.
/// Cues are written in the order given; their `index` field is honored, so call
/// [`renumber`] first if you need 1..N numbering.
pub fn to_srt(cues: &[Cue]) -> String {
    let mut out = String::new();
    for cue in cues {
        out.push_str(&cue.index.to_string());
        out.push('\n');
        out.push_str(&cue.start.to_srt());
        out.push_str(" --> ");
        out.push_str(&cue.end.to_srt());
        out.push('\n');
        if !cue.text.is_empty() {
            out.push_str(&cue.text);
            out.push('\n');
        }
        out.push('\n');
    }
    out
}

/// Serialize cues to WebVTT text. Emits the mandatory `WEBVTT` header followed
/// by a blank line, then each cue with `.`-style millisecond separators.
pub fn to_vtt(cues: &[Cue]) -> String {
    let mut out = String::from("WEBVTT\n\n");
    for cue in cues {
        // VTT cue identifiers are optional; emit the numeric index for parity.
        out.push_str(&cue.index.to_string());
        out.push('\n');
        out.push_str(&cue.start.to_vtt());
        out.push_str(" --> ");
        out.push_str(&cue.end.to_vtt());
        out.push('\n');
        if !cue.text.is_empty() {
            out.push_str(&cue.text);
            out.push('\n');
        }
        out.push('\n');
    }
    out
}

/// Serialize to the given [`Format`].
pub fn to_format(cues: &[Cue], format: Format) -> String {
    match format {
        Format::Srt => to_srt(cues),
        Format::Vtt => to_vtt(cues),
    }
}

/// Rewrite `index` fields to 1..N in current vector order.
pub fn renumber(cues: &mut [Cue]) {
    for (i, cue) in cues.iter_mut().enumerate() {
        cue.index = i + 1;
    }
}

/// Shift every cue (start and end) by `delta_ms`. If `from` is given, only cues
/// whose start is at/after `from` are moved. Times saturate at zero.
pub fn shift(cues: &mut [Cue], delta_ms: i64, from: Option<Timestamp>) {
    for cue in cues.iter_mut() {
        if let Some(threshold) = from {
            if cue.start < threshold {
                continue;
            }
        }
        cue.start = cue.start.shifted(delta_ms);
        cue.end = cue.end.shifted(delta_ms);
    }
}

/// Summary statistics for a cue list, produced by [`stats`].
///
/// `span_ms` is wall-clock from the earliest start to the latest end;
/// `display_ms` is the sum of every cue's on-screen duration (negative
/// durations counted as zero); `covered_ms` is the length of the *union* of
/// on-screen intervals (overlaps counted once). `coverage()` relates the union
/// to the span.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stats {
    /// Number of cues.
    pub count: usize,
    /// Earliest start across all cues (`None` for an empty list).
    pub first_start: Option<Timestamp>,
    /// Latest end across all cues (`None` for an empty list).
    pub last_end: Option<Timestamp>,
    /// Latest end minus earliest start, in milliseconds (0 for an empty list).
    pub span_ms: i64,
    /// Sum of on-screen durations, in milliseconds (negatives clamped to 0).
    /// This double-counts time when cues overlap; use `covered_ms` for the union.
    pub display_ms: i64,
    /// Length of the union of on-screen intervals, in milliseconds (overlapping
    /// cues counted once, negative durations counted as zero). Always `<= span_ms`.
    pub covered_ms: i64,
}

impl Stats {
    /// Fraction of the span during which at least one subtitle is on screen, in
    /// `0.0..=1.0` (0.0 when the span is zero). Because it uses the union of
    /// on-screen intervals (`covered_ms`), overlapping cues are never
    /// double-counted, so the value naturally stays at or below 1.0.
    pub fn coverage(&self) -> f64 {
        if self.span_ms <= 0 {
            0.0
        } else {
            (self.covered_ms as f64 / self.span_ms as f64).min(1.0)
        }
    }
}

/// Compute summary [`Stats`] over a cue list without modifying it. Cues need not
/// be sorted; the span is taken from the minimum start to the maximum end.
pub fn stats(cues: &[Cue]) -> Stats {
    let first_start = cues.iter().map(|c| c.start).min();
    let last_end = cues.iter().map(|c| c.end).max();
    let span_ms = match (first_start, last_end) {
        (Some(s), Some(e)) => (e.ms - s.ms).max(0),
        _ => 0,
    };
    let display_ms = cues.iter().map(|c| c.duration_ms().max(0)).sum();
    let covered_ms = union_ms(cues);
    Stats {
        count: cues.len(),
        first_start,
        last_end,
        span_ms,
        display_ms,
        covered_ms,
    }
}

/// Length of the union of every cue's on-screen interval, in milliseconds.
/// Overlapping cues are counted once; a cue with `end < start` is treated as a
/// zero-length interval (so it contributes nothing).
fn union_ms(cues: &[Cue]) -> i64 {
    let mut intervals: Vec<(i64, i64)> = cues
        .iter()
        .map(|c| (c.start.ms, c.end.ms.max(c.start.ms)))
        .collect();
    intervals.sort_by_key(|&(start, _)| start);

    let mut total = 0;
    let mut cur: Option<(i64, i64)> = None;
    for (start, end) in intervals {
        match cur {
            Some((cur_start, cur_end)) if start <= cur_end => {
                cur = Some((cur_start, cur_end.max(end)));
            }
            Some((cur_start, cur_end)) => {
                total += cur_end - cur_start;
                cur = Some((start, end));
            }
            None => cur = Some((start, end)),
        }
    }
    if let Some((cur_start, cur_end)) = cur {
        total += cur_end - cur_start;
    }
    total
}

/// Linearly scale every timestamp by `factor` (e.g. 23.976→25 fps drift).
pub fn scale(cues: &mut [Cue], factor: f64) {
    for cue in cues.iter_mut() {
        cue.start = cue.start.scaled(factor);
        cue.end = cue.end.scaled(factor);
    }
}

/// Concatenate multiple cue lists into one, renumbering 1..N. `offset_ms`, if
/// non-zero, is applied cumulatively: the k-th file (0-based) is shifted by
/// `k * offset_ms`. (A common use is to insert a fixed gap between joined
/// files.) Cue *times* are otherwise preserved.
pub fn merge(lists: &[Vec<Cue>], offset_ms: i64) -> Vec<Cue> {
    let mut out = Vec::new();
    for (k, list) in lists.iter().enumerate() {
        let delta = offset_ms * k as i64;
        for cue in list {
            let mut c = cue.clone();
            if delta != 0 {
                c.start = c.start.shifted(delta);
                c.end = c.end.shifted(delta);
            }
            out.push(c);
        }
    }
    renumber(&mut out);
    out
}

/// Clean up a cue list:
///   * drop empty cues,
///   * sort by start time (stable),
///   * clamp overlaps so each cue's end is at most the next cue's start,
///   * ensure end >= start for every cue,
///   * renumber 1..N.
///
/// Returns the cleaned list (the input is consumed).
pub fn fix(mut cues: Vec<Cue>) -> Vec<Cue> {
    // Drop empties.
    cues.retain(|c| !c.is_empty());

    // Ensure end >= start within each cue first.
    for c in cues.iter_mut() {
        if c.end < c.start {
            c.end = c.start;
        }
    }

    // Stable sort by start, then by end.
    cues.sort_by(|a, b| a.start.cmp(&b.start).then(a.end.cmp(&b.end)));

    // Clamp overlaps: a cue may not extend past the next cue's start.
    for i in 0..cues.len().saturating_sub(1) {
        let next_start = cues[i + 1].start;
        if cues[i].end > next_start {
            cues[i].end = next_start;
        }
    }

    renumber(&mut cues);
    cues
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "1\n00:00:01,000 --> 00:00:04,000\nHello world\n\n2\n00:00:05,500 --> 00:00:07,250\nSecond line\nwith two rows\n";

    #[test]
    fn timestamp_parse_srt() {
        let t = Timestamp::parse("01:02:03,456").unwrap();
        assert_eq!(t.ms, ((1 * 60 + 2) * 60 + 3) * 1000 + 456);
        assert_eq!(t.to_srt(), "01:02:03,456");
    }

    #[test]
    fn timestamp_parse_vtt_dot_and_no_hours() {
        let t = Timestamp::parse("02:03.456").unwrap();
        assert_eq!(t, Timestamp::from_hmsm(0, 2, 3, 456));
        assert_eq!(t.to_vtt(), "00:02:03.456");
    }

    #[test]
    fn timestamp_parse_short_millis_padding() {
        // "5" tenths means 500ms; "05" means 50ms.
        assert_eq!(Timestamp::parse("00:00:00,5").unwrap().ms, 500);
        assert_eq!(Timestamp::parse("00:00:00,05").unwrap().ms, 50);
        assert_eq!(Timestamp::parse("00:00:00,005").unwrap().ms, 5);
    }

    #[test]
    fn timestamp_parse_rejects_garbage() {
        assert!(Timestamp::parse("aa:bb:cc,ddd").is_err());
        assert!(Timestamp::parse("00:99:00,000").is_err()); // minutes out of range
        assert!(Timestamp::parse("").is_err());
    }

    #[test]
    fn parse_known_srt() {
        let cues = parse(SAMPLE).unwrap();
        assert_eq!(cues.len(), 2);
        assert_eq!(cues[0].index, 1);
        assert_eq!(cues[0].start, Timestamp::from_hmsm(0, 0, 1, 0));
        assert_eq!(cues[0].end, Timestamp::from_hmsm(0, 0, 4, 0));
        assert_eq!(cues[0].text, "Hello world");
        assert_eq!(cues[1].text, "Second line\nwith two rows");
        assert_eq!(cues[1].start, Timestamp::from_hmsm(0, 0, 5, 500));
    }

    #[test]
    fn parse_crlf_and_bom() {
        let with_bom = "\u{feff}1\r\n00:00:01,000 --> 00:00:02,000\r\nHi\r\n\r\n";
        let cues = parse(with_bom).unwrap();
        assert_eq!(cues.len(), 1);
        assert_eq!(cues[0].text, "Hi");
    }

    #[test]
    fn shift_by_2500ms_exact() {
        let mut cues = parse(SAMPLE).unwrap();
        shift(&mut cues, 2500, None);
        // 00:00:01,000 + 2.5s = 00:00:03,500
        assert_eq!(cues[0].start.to_srt(), "00:00:03,500");
        // 00:00:04,000 + 2.5s = 00:00:06,500
        assert_eq!(cues[0].end.to_srt(), "00:00:06,500");
        // 00:00:05,500 + 2.5s = 00:00:08,000
        assert_eq!(cues[1].start.to_srt(), "00:00:08,000");
        // 00:00:07,250 + 2.5s = 00:00:09,750
        assert_eq!(cues[1].end.to_srt(), "00:00:09,750");
    }

    #[test]
    fn shift_negative_saturates_at_zero() {
        let mut cues = parse(SAMPLE).unwrap();
        shift(&mut cues, -10_000, None);
        assert_eq!(cues[0].start.ms, 0);
        assert_eq!(cues[0].end.ms, 0);
    }

    #[test]
    fn shift_from_threshold_only_moves_later_cues() {
        let mut cues = parse(SAMPLE).unwrap();
        let from = Timestamp::from_hmsm(0, 0, 5, 0);
        shift(&mut cues, 1000, Some(from));
        // First cue (starts at 1s) is untouched.
        assert_eq!(cues[0].start.to_srt(), "00:00:01,000");
        // Second cue (starts at 5.5s >= 5s) moves +1s.
        assert_eq!(cues[1].start.to_srt(), "00:00:06,500");
    }

    #[test]
    fn stats_summarizes_sample() {
        let cues = parse(SAMPLE).unwrap();
        let s = stats(&cues);
        assert_eq!(s.count, 2);
        assert_eq!(s.first_start, Some(Timestamp::from_hmsm(0, 0, 1, 0)));
        assert_eq!(s.last_end, Some(Timestamp::from_hmsm(0, 0, 7, 250)));
        // Span: 1.000 -> 7.250 = 6250ms.
        assert_eq!(s.span_ms, 6250);
        // On-screen: (4.000-1.000) + (7.250-5.500) = 3000 + 1750 = 4750ms.
        assert_eq!(s.display_ms, 4750);
        // Coverage: 4750 / 6250 = 0.76.
        assert!((s.coverage() - 0.76).abs() < 1e-9);
    }

    #[test]
    fn stats_handles_unsorted_and_overlapping_cues() {
        // Out of order; the second cue overlaps and extends past the first.
        let cues = vec![
            Cue::new(1, Timestamp::from_ms(5000), Timestamp::from_ms(6000), "b"),
            Cue::new(2, Timestamp::from_ms(1000), Timestamp::from_ms(8000), "a"),
        ];
        let s = stats(&cues);
        assert_eq!(s.first_start, Some(Timestamp::from_ms(1000)));
        assert_eq!(s.last_end, Some(Timestamp::from_ms(8000)));
        assert_eq!(s.span_ms, 7000);
        // Display sum (1000 + 7000 = 8000) exceeds the span -> coverage clamps to 1.
        assert_eq!(s.display_ms, 8000);
        assert_eq!(s.coverage(), 1.0);
    }

    #[test]
    fn stats_empty_list_is_zeroed() {
        let s = stats(&[]);
        assert_eq!(s.count, 0);
        assert_eq!(s.first_start, None);
        assert_eq!(s.last_end, None);
        assert_eq!(s.span_ms, 0);
        assert_eq!(s.display_ms, 0);
        assert_eq!(s.covered_ms, 0);
        assert_eq!(s.coverage(), 0.0);
    }

    #[test]
    fn coverage_uses_union_not_sum_for_overlaps() {
        // Two identical 0-10s cues fully overlap; a third runs 50-60s. Span 0-60s.
        let cues = vec![
            Cue::new(1, Timestamp::from_ms(0), Timestamp::from_ms(10_000), "a"),
            Cue::new(2, Timestamp::from_ms(0), Timestamp::from_ms(10_000), "b"),
            Cue::new(
                3,
                Timestamp::from_ms(50_000),
                Timestamp::from_ms(60_000),
                "c",
            ),
        ];
        let s = stats(&cues);
        assert_eq!(s.span_ms, 60_000);
        // Summed on-screen double-counts the overlap: 10 + 10 + 10 = 30s.
        assert_eq!(s.display_ms, 30_000);
        // Union counts the overlap once: 10s + 10s = 20s.
        assert_eq!(s.covered_ms, 20_000);
        // 20 / 60 = 33.3%, not 50% (which sum/span would give).
        assert!((s.coverage() - 1.0 / 3.0).abs() < 1e-9);
    }

    #[test]
    fn stats_clamps_negative_duration_cue() {
        // A reversed cue (end < start) must contribute zero, not negative, to
        // both the summed and the union on-screen totals.
        let cues = vec![
            Cue::new(1, Timestamp::from_ms(5000), Timestamp::from_ms(2000), "rev"),
            Cue::new(2, Timestamp::from_ms(6000), Timestamp::from_ms(8000), "ok"),
        ];
        let s = stats(&cues);
        // Span: earliest start 5000 -> latest end 8000 = 3000ms.
        assert_eq!(s.span_ms, 3000);
        // Reversed cue clamped to 0, plus the 2000ms real cue.
        assert_eq!(s.display_ms, 2000);
        assert_eq!(s.covered_ms, 2000);
        assert!((s.coverage() - 2000.0 / 3000.0).abs() < 1e-9);
    }

    #[test]
    fn scale_exact() {
        let mut cues = parse(SAMPLE).unwrap();
        scale(&mut cues, 2.0);
        // 1000ms * 2 = 2000ms
        assert_eq!(cues[0].start.to_srt(), "00:00:02,000");
        // 4000 * 2 = 8000
        assert_eq!(cues[0].end.to_srt(), "00:00:08,000");
        // 7250 * 2 = 14500
        assert_eq!(cues[1].end.to_srt(), "00:00:14,500");
    }

    #[test]
    fn scale_rounds_to_nearest_ms() {
        // 1000ms * 1.0005 = 1000.5 -> rounds to 1001 (round half away from zero).
        let mut cues = vec![Cue::new(
            1,
            Timestamp::from_ms(1000),
            Timestamp::from_ms(2000),
            "x",
        )];
        scale(&mut cues, 1.0005);
        assert_eq!(cues[0].start.ms, 1001);
        assert_eq!(cues[0].end.ms, 2001);
    }

    #[test]
    fn fix_reorders_and_renumbers_scrambled_input() {
        // Out of order, with bad indices, an empty cue, and an overlap.
        let scrambled = vec![
            Cue::new(
                7,
                Timestamp::from_ms(5000),
                Timestamp::from_ms(6000),
                "third",
            ),
            Cue::new(
                2,
                Timestamp::from_ms(1000),
                Timestamp::from_ms(3500),
                "first",
            ),
            Cue::new(3, Timestamp::from_ms(9000), Timestamp::from_ms(9000), "   "), // empty
            Cue::new(
                4,
                Timestamp::from_ms(3000),
                Timestamp::from_ms(4000),
                "second",
            ),
        ];
        let fixed = fix(scrambled);
        assert_eq!(fixed.len(), 3); // empty dropped
        assert_eq!(fixed[0].index, 1);
        assert_eq!(fixed[0].text, "first");
        assert_eq!(fixed[1].index, 2);
        assert_eq!(fixed[1].text, "second");
        assert_eq!(fixed[2].index, 3);
        assert_eq!(fixed[2].text, "third");
        // "first" ended at 3500 but "second" starts at 3000 -> clamped to 3000.
        assert_eq!(fixed[0].end.ms, 3000);
    }

    #[test]
    fn fix_clamps_end_before_start() {
        let bad = vec![Cue::new(
            1,
            Timestamp::from_ms(5000),
            Timestamp::from_ms(2000),
            "rev",
        )];
        let fixed = fix(bad);
        assert_eq!(fixed[0].end.ms, 5000); // end clamped up to start
    }

    #[test]
    fn merge_concatenates_and_renumbers() {
        let a = parse(SAMPLE).unwrap();
        let b = parse(SAMPLE).unwrap();
        let merged = merge(&[a, b], 0);
        assert_eq!(merged.len(), 4);
        assert_eq!(merged[0].index, 1);
        assert_eq!(merged[3].index, 4);
        // Times preserved (offset 0): 3rd cue == first sample's first cue time.
        assert_eq!(merged[2].start.to_srt(), "00:00:01,000");
    }

    #[test]
    fn merge_with_offset_shifts_subsequent_files() {
        let a = parse(SAMPLE).unwrap();
        let b = parse(SAMPLE).unwrap();
        let merged = merge(&[a, b], 60_000); // +60s between files
                                             // File 0 unchanged.
        assert_eq!(merged[0].start.to_srt(), "00:00:01,000");
        // File 1 (k=1) shifted +60s: 1s -> 61s.
        assert_eq!(merged[2].start.to_srt(), "00:01:01,000");
    }

    #[test]
    fn srt_to_vtt_round_trip_preserves_cues() {
        let original = parse(SAMPLE).unwrap();
        let vtt = to_vtt(&original);
        assert!(vtt.starts_with("WEBVTT"));
        let back = parse(&vtt).unwrap();
        assert_eq!(original, back);
    }

    #[test]
    fn vtt_to_srt_round_trip_preserves_cues() {
        let vtt = "WEBVTT\n\nNOTE this is a note\n\nintro\n00:00:01.000 --> 00:00:04.000\nHello\n\n00:00:05.500 --> 00:00:07.250 position:50%\nWorld\n";
        let cues = parse(vtt).unwrap();
        assert_eq!(cues.len(), 2);
        assert_eq!(cues[0].text, "Hello");
        assert_eq!(cues[1].start, Timestamp::from_hmsm(0, 0, 5, 500));
        // Cue settings on the timing line are ignored.
        assert_eq!(cues[1].end, Timestamp::from_hmsm(0, 0, 7, 250));
        let srt = to_srt(&cues);
        let reparsed = parse(&srt).unwrap();
        assert_eq!(cues, reparsed);
    }

    #[test]
    fn parse_empty_input_errors() {
        assert_eq!(parse("\n\n  \n"), Err(ParseError::Empty));
    }

    #[test]
    fn parse_missing_arrow_errors() {
        let bad = "1\nthis is not a timing line\nHello\n";
        assert!(matches!(parse(bad), Err(ParseError::MissingArrow { .. })));
    }

    #[test]
    fn format_from_path_infers_extension() {
        assert_eq!(Format::from_path("a.vtt"), Format::Vtt);
        assert_eq!(Format::from_path("A.VTT"), Format::Vtt);
        assert_eq!(Format::from_path("a.srt"), Format::Srt);
        assert_eq!(Format::from_path("noext"), Format::Srt);
    }
}
