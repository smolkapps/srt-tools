//! Integration tests that drive the real `srt-tools` binary.

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use tempfile::tempdir;

const SAMPLE: &str = "1\n00:00:01,000 --> 00:00:04,000\nHello world\n\n2\n00:00:05,500 --> 00:00:07,250\nSecond line\nwith two rows\n";

fn bin() -> Command {
    Command::cargo_bin("srt-tools").expect("binary builds")
}

#[test]
fn shift_writes_expected_timestamps_to_file() {
    let dir = tempdir().unwrap();
    let input = dir.path().join("in.srt");
    let output = dir.path().join("out.srt");
    fs::write(&input, SAMPLE).unwrap();

    bin()
        .args([
            "shift",
            input.to_str().unwrap(),
            "--by",
            "+2.5s",
            "-o",
            output.to_str().unwrap(),
        ])
        .assert()
        .success();

    let out = fs::read_to_string(&output).unwrap();
    assert!(out.contains("00:00:03,500 --> 00:00:06,500"), "got:\n{out}");
    assert!(out.contains("00:00:08,000 --> 00:00:09,750"), "got:\n{out}");
}

#[test]
fn shift_accepts_negative_and_timestamp_form() {
    let dir = tempdir().unwrap();
    let input = dir.path().join("in.srt");
    fs::write(&input, SAMPLE).unwrap();

    // -1.2s on cue1 start 1.000 -> 0.000 (saturates? 1.000-1.2 = -0.2 -> 0)
    let out = bin()
        .args(["shift", input.to_str().unwrap(), "--by", "-1.2s"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let text = String::from_utf8(out).unwrap();
    assert!(
        text.contains("00:00:00,000 --> 00:00:02,800"),
        "got:\n{text}"
    );

    // Timestamp-form shift +00:00:02,500
    let out2 = bin()
        .args(["shift", input.to_str().unwrap(), "--by", "00:00:02,500"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let text2 = String::from_utf8(out2).unwrap();
    assert!(
        text2.contains("00:00:03,500 --> 00:00:06,500"),
        "got:\n{text2}"
    );
}

#[test]
fn shift_from_threshold_via_cli() {
    let dir = tempdir().unwrap();
    let input = dir.path().join("in.srt");
    fs::write(&input, SAMPLE).unwrap();

    let out = bin()
        .args([
            "shift",
            input.to_str().unwrap(),
            "--by",
            "+1s",
            "--from",
            "00:00:05,000",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let text = String::from_utf8(out).unwrap();
    // First cue unchanged, second cue +1s.
    assert!(
        text.contains("00:00:01,000 --> 00:00:04,000"),
        "got:\n{text}"
    );
    assert!(
        text.contains("00:00:06,500 --> 00:00:08,250"),
        "got:\n{text}"
    );
}

#[test]
fn convert_srt_to_vtt_by_extension() {
    let dir = tempdir().unwrap();
    let input = dir.path().join("in.srt");
    let output = dir.path().join("out.vtt");
    fs::write(&input, SAMPLE).unwrap();

    bin()
        .args([
            "convert",
            input.to_str().unwrap(),
            "-o",
            output.to_str().unwrap(),
        ])
        .assert()
        .success();

    let out = fs::read_to_string(&output).unwrap();
    assert!(out.starts_with("WEBVTT"), "got:\n{out}");
    assert!(out.contains("00:00:01.000 --> 00:00:04.000"), "got:\n{out}");
}

#[test]
fn convert_vtt_to_srt_round_trips() {
    let dir = tempdir().unwrap();
    let vtt = dir.path().join("in.vtt");
    let srt = dir.path().join("out.srt");
    fs::write(
        &vtt,
        "WEBVTT\n\n00:00:01.000 --> 00:00:04.000\nHello\n\n00:00:05.500 --> 00:00:07.250\nWorld\n",
    )
    .unwrap();

    bin()
        .args([
            "convert",
            vtt.to_str().unwrap(),
            "-o",
            srt.to_str().unwrap(),
        ])
        .assert()
        .success();

    let out = fs::read_to_string(&srt).unwrap();
    assert!(out.contains("00:00:01,000 --> 00:00:04,000"), "got:\n{out}");
    assert!(out.contains("Hello"));
    assert!(out.contains("World"));
}

#[test]
fn merge_concatenates_and_renumbers() {
    let dir = tempdir().unwrap();
    let a = dir.path().join("a.srt");
    let b = dir.path().join("b.srt");
    let out = dir.path().join("out.srt");
    fs::write(&a, SAMPLE).unwrap();
    fs::write(&b, SAMPLE).unwrap();

    bin()
        .args([
            "merge",
            a.to_str().unwrap(),
            b.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
        ])
        .assert()
        .success();

    let text = fs::read_to_string(&out).unwrap();
    // Four cues, indices 1..4 present.
    assert!(text.contains("\n4\n") || text.starts_with("4\n") || text.contains("\n4\r\n"));
    let count = text.matches("-->").count();
    assert_eq!(count, 4, "expected 4 cues, got {count} in:\n{text}");
}

#[test]
fn merge_with_offset_shifts_second_file() {
    let dir = tempdir().unwrap();
    let a = dir.path().join("a.srt");
    let b = dir.path().join("b.srt");
    let out = dir.path().join("out.srt");
    fs::write(&a, SAMPLE).unwrap();
    fs::write(&b, SAMPLE).unwrap();

    bin()
        .args([
            "merge",
            a.to_str().unwrap(),
            b.to_str().unwrap(),
            "--offset",
            "1m",
            "-o",
            out.to_str().unwrap(),
        ])
        .assert()
        .success();

    let text = fs::read_to_string(&out).unwrap();
    // Second file's first cue: 1s + 60s = 61s = 00:01:01,000.
    assert!(
        text.contains("00:01:01,000 --> 00:01:04,000"),
        "got:\n{text}"
    );
}

#[test]
fn fix_sorts_renumbers_and_drops_empty() {
    let dir = tempdir().unwrap();
    let input = dir.path().join("scrambled.srt");
    let output = dir.path().join("fixed.srt");
    // Out of order indices/times, an empty cue body, overlapping times.
    let scrambled = "9\n00:00:05,000 --> 00:00:06,000\nthird\n\n2\n00:00:01,000 --> 00:00:03,500\nfirst\n\n5\n00:00:09,000 --> 00:00:10,000\n\n\n4\n00:00:03,000 --> 00:00:04,000\nsecond\n";
    fs::write(&input, scrambled).unwrap();

    bin()
        .args([
            "fix",
            input.to_str().unwrap(),
            "-o",
            output.to_str().unwrap(),
        ])
        .assert()
        .success();

    let text = fs::read_to_string(&output).unwrap();
    // 3 cues remain (empty dropped).
    assert_eq!(text.matches("-->").count(), 3, "got:\n{text}");
    // Sorted order: first, second, third.
    let first = text.find("first").unwrap();
    let second = text.find("second").unwrap();
    let third = text.find("third").unwrap();
    assert!(first < second && second < third, "order wrong:\n{text}");
    // Overlap clamp: "first" ended 3.500 but "second" starts 3.000.
    assert!(
        text.contains("00:00:01,000 --> 00:00:03,000"),
        "got:\n{text}"
    );
    // Renumbered 1,2,3.
    assert!(text.starts_with("1\n"), "got:\n{text}");
}

#[test]
fn stats_reports_counts_and_coverage() {
    let dir = tempdir().unwrap();
    let input = dir.path().join("in.srt");
    fs::write(&input, SAMPLE).unwrap();

    let out = bin()
        .args(["stats", input.to_str().unwrap()])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let text = String::from_utf8(out).unwrap();
    // Two cues in SAMPLE.
    assert!(text.contains("cues:      2"), "got:\n{text}");
    // Span 1.000 -> 7.250 = 6.250.
    assert!(text.contains("span:      00:00:06,250"), "got:\n{text}");
    // On-screen (4.000-1.000)+(7.250-5.500) = 4.750.
    assert!(text.contains("on-screen: 00:00:04,750"), "got:\n{text}");
    // Coverage 4750/6250 = 76.0%.
    assert!(text.contains("coverage:  76.0%"), "got:\n{text}");
}

#[test]
fn stats_reads_from_stdin() {
    let out = bin()
        .args(["stats"])
        .write_stdin(SAMPLE)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("cues:      2"), "got:\n{text}");
    assert!(text.contains("first:     00:00:01,000"), "got:\n{text}");
    assert!(text.contains("last:      00:00:07,250"), "got:\n{text}");
}

#[test]
fn stats_on_cueless_file_reports_zero() {
    let dir = tempdir().unwrap();
    let input = dir.path().join("empty.srt");
    // No cues at all (only blank lines) must report zero, not exit non-zero.
    fs::write(&input, "   \n\n").unwrap();

    let out = bin()
        .args(["stats", input.to_str().unwrap()])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("cues:      0"), "got:\n{text}");
    assert!(text.contains("first:     -"), "got:\n{text}");
    assert!(text.contains("last:      -"), "got:\n{text}");
    assert!(text.contains("coverage:  0.0%"), "got:\n{text}");
}

#[test]
fn scale_doubles_timestamps() {
    let dir = tempdir().unwrap();
    let input = dir.path().join("in.srt");
    let output = dir.path().join("out.srt");
    fs::write(&input, SAMPLE).unwrap();

    bin()
        .args([
            "scale",
            input.to_str().unwrap(),
            "--factor",
            "2.0",
            "-o",
            output.to_str().unwrap(),
        ])
        .assert()
        .success();

    let text = fs::read_to_string(&output).unwrap();
    assert!(
        text.contains("00:00:02,000 --> 00:00:08,000"),
        "got:\n{text}"
    );
    assert!(
        text.contains("00:00:11,000 --> 00:00:14,500"),
        "got:\n{text}"
    );
}

#[test]
fn stdin_to_stdout_pipeline() {
    let out = bin()
        .args(["shift", "--by", "+1s"])
        .write_stdin(SAMPLE)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let text = String::from_utf8(out).unwrap();
    assert!(
        text.contains("00:00:02,000 --> 00:00:05,000"),
        "got:\n{text}"
    );
}

#[test]
fn malformed_input_exits_nonzero_with_clear_stderr() {
    let dir = tempdir().unwrap();
    let input = dir.path().join("bad.srt");
    // A cue index with no timing line after it.
    fs::write(&input, "1\nthis line has no arrow\nsome text\n").unwrap();

    bin()
        .args(["shift", input.to_str().unwrap(), "--by", "+1s"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("error:").and(predicate::str::contains("parse")));
}

#[test]
fn empty_input_exits_nonzero() {
    bin()
        .args(["fix", "--by", "+1s"]) // wrong flag too, but empty stdin is the point
        .write_stdin("   \n\n")
        .assert()
        .failure();
}

#[test]
fn missing_file_exits_nonzero_with_path_in_error() {
    bin()
        .args(["shift", "/no/such/file_12345.srt", "--by", "+1s"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("error:"));
}

#[test]
fn bad_duration_exits_nonzero() {
    bin()
        .args(["shift", "--by", "banana"])
        .write_stdin(SAMPLE)
        .assert()
        .failure()
        .stderr(predicate::str::contains("error:"));
}

#[test]
fn convert_without_target_format_errors() {
    // No --to and stdout (no -o extension) => cannot infer format.
    bin()
        .args(["convert"])
        .write_stdin(SAMPLE)
        .assert()
        .failure()
        .stderr(predicate::str::contains("convert needs"));
}

#[test]
fn negative_scale_factor_rejected() {
    bin()
        .args(["scale", "--factor", "-1.0"])
        .write_stdin(SAMPLE)
        .assert()
        .failure()
        .stderr(predicate::str::contains("--factor"));
}
