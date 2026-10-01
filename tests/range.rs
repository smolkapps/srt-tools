//! Numeric validation must finish before touching input or output files.

use assert_cmd::Command;
use predicates::prelude::*;
use srt_tools::{parse, to_format, Cue, Format, Timestamp};
use std::fs;
use tempfile::tempdir;

fn bin() -> Command {
    Command::cargo_bin("srt-tools").expect("binary builds")
}

fn sample(format: Format) -> String {
    to_format(
        &[
            Cue::new(
                1,
                Timestamp::from_ms(1000),
                Timestamp::from_ms(4000),
                "Héllo 世界\nsecond line",
            ),
            Cue::new(
                2,
                Timestamp::from_ms(3000),
                Timestamp::from_ms(5000),
                "Привет 👋",
            ),
        ],
        format,
    )
}

#[test]
fn invalid_durations_preserve_files_and_create_no_output() {
    let dir = tempdir().unwrap();
    for (format, ext) in [(Format::Srt, "srt"), (Format::Vtt, "vtt")] {
        let source = sample(format);
        let input = dir.path().join(format!("in.{ext}"));
        let output = dir.path().join(format!("out.{ext}"));
        let new_output = dir.path().join(format!("new-{ext}/out.{ext}"));
        fs::write(&input, &source).unwrap();
        for by in [
            "NaN",
            "-NaNms",
            "inf",
            "-inf",
            "+infs",
            "infinitym",
            "1e300",
            "-1e300ms",
            "1e300s",
            "1e300m",
            "1e309s",
            "9223372036854775808ms",
            "-9223372036854775809ms",
            "-9223372036854775809.0ms",
            "-9.223372036854776e18ms",
            "9223372036854776s",
            "153722867280913m",
            "2562047788015:12:55,808",
            "9223372036854775807:00:00",
        ] {
            fs::write(&output, "OUTPUT SENTINEL\n").unwrap();
            for destination in [&output, &new_output, &input] {
                bin()
                    .args([
                        "shift",
                        input.to_str().unwrap(),
                        "--by",
                        by,
                        "-o",
                        destination.to_str().unwrap(),
                    ])
                    .assert()
                    .code(1)
                    .stdout("")
                    .stderr(predicate::str::contains("invalid --by duration"));
            }
            assert_eq!(fs::read_to_string(&input).unwrap(), source, "{by}");
            assert_eq!(
                fs::read_to_string(&output).unwrap(),
                "OUTPUT SENTINEL\n",
                "{by}"
            );
            assert!(!new_output.parent().unwrap().exists(), "{by}");
        }
    }
}

#[test]
fn shift_endpoint_overflow_is_rejected_before_writing() {
    let dir = tempdir().unwrap();
    let input = dir.path().join("in.srt");
    let output = dir.path().join("out.srt");
    // The start fits after shifting; the end does not.
    let source = to_format(
        &[Cue::new(
            1,
            Timestamp::from_ms(0),
            Timestamp::from_ms(1),
            "boundary",
        )],
        Format::Srt,
    );
    fs::write(&input, &source).unwrap();
    fs::write(&output, "SENTINEL").unwrap();
    bin()
        .args([
            "shift",
            input.to_str().unwrap(),
            "--by",
            "9223372036854775807ms",
            "-o",
            output.to_str().unwrap(),
        ])
        .assert()
        .code(1)
        .stdout("")
        .stderr(predicate::str::contains("timestamp range"));
    assert_eq!(fs::read_to_string(&input).unwrap(), source);
    assert_eq!(fs::read_to_string(&output).unwrap(), "SENTINEL");
}

#[test]
fn exact_duration_boundaries_and_negative_clamp_are_supported() {
    for by in ["9223372036854775807ms", "2562047788015:12:55,807"] {
        let result = bin()
            .args(["shift", "--by", by])
            .write_stdin("1\n00:00:00,000 --> 00:00:00,000\nboundary\n")
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        let cues = parse(std::str::from_utf8(&result).unwrap()).unwrap();
        assert_eq!(cues[0].start.ms, i64::MAX, "{by}");
        assert_eq!(cues[0].end.ms, i64::MAX, "{by}");
    }
    for by in ["-9223372036854775808ms", "-2562047788015:12:55,807"] {
        let result = bin()
            .args(["shift", "--by", by])
            .write_stdin(sample(Format::Srt))
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        let cues = parse(std::str::from_utf8(&result).unwrap()).unwrap();
        assert!(
            cues.iter().all(|cue| cue.start.ms == 0 && cue.end.ms == 0),
            "{by}"
        );
    }
}

#[test]
fn threshold_skips_overflowing_endpoints_of_excluded_cues() {
    let source = to_format(
        &[Cue::new(
            1,
            Timestamp::from_ms(i64::MAX - 1),
            Timestamp::from_ms(i64::MAX),
            "excluded",
        )],
        Format::Srt,
    );
    let result = bin()
        .args(["shift", "--by", "1ms", "--from", "2562047788015:12:55,807"])
        .write_stdin(source.clone())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    assert_eq!(std::str::from_utf8(&result).unwrap(), source);
}

#[test]
fn merge_rejects_invalid_and_cumulative_offsets_before_output() {
    let dir = tempdir().unwrap();
    let input = dir.path().join("in.vtt");
    let output = dir.path().join("out.vtt");
    let new_output = dir.path().join("new-merge/out.vtt");
    let source = sample(Format::Vtt);
    fs::write(&input, &source).unwrap();
    for offset in [
        "NaN",
        "inf",
        "-inf",
        "1e300m",
        "9223372036854775808ms",
        "9223372036854775807ms",
        "4611686018427387904ms",
        "-4611686018427387905ms",
    ] {
        fs::write(&output, "SENTINEL").unwrap();
        for destination in [&output, &new_output, &input] {
            bin()
                .args([
                    "merge",
                    input.to_str().unwrap(),
                    input.to_str().unwrap(),
                    input.to_str().unwrap(),
                    "--offset",
                    offset,
                    "-o",
                    destination.to_str().unwrap(),
                ])
                .assert()
                .code(1)
                .stdout("")
                .stderr(predicate::str::contains("error:"));
        }
        assert_eq!(fs::read_to_string(&output).unwrap(), "SENTINEL", "{offset}");
        assert_eq!(fs::read_to_string(&input).unwrap(), source, "{offset}");
        assert!(!new_output.parent().unwrap().exists(), "{offset}");
    }
}

#[test]
fn oversized_input_and_threshold_timestamps_are_parse_errors() {
    let dir = tempdir().unwrap();
    let output = dir.path().join("new/out.srt");
    for (ext, arrow) in [
        ("srt", "2562047788015:12:55,808 --> 2562047788015:12:55,808"),
        (
            "vtt",
            "9223372036854775807:00:00.000 --> 9223372036854775807:00:00.000",
        ),
    ] {
        let input = dir.path().join(format!("bad.{ext}"));
        let header = if ext == "vtt" { "WEBVTT\n\n" } else { "" };
        let source = format!("{header}1\n{arrow}\nsynthetic\n");
        fs::write(&input, &source).unwrap();
        bin()
            .args([
                "shift",
                input.to_str().unwrap(),
                "--by",
                "1ms",
                "-o",
                output.to_str().unwrap(),
            ])
            .assert()
            .code(1)
            .stderr(predicate::str::contains("failed to parse subtitles"));
        assert_eq!(fs::read_to_string(input).unwrap(), source);
        assert!(!output.parent().unwrap().exists());
    }
    bin()
        .args([
            "shift",
            "--by",
            "1ms",
            "--from",
            "2562047788015:12:55,808",
            "-o",
            output.to_str().unwrap(),
        ])
        .write_stdin(sample(Format::Srt))
        .assert()
        .code(1)
        .stderr(predicate::str::contains("invalid --from timestamp"));
    assert!(!output.parent().unwrap().exists());
}

#[test]
fn ordinary_shifts_preserve_unicode_multiline_and_overlap_semantics() {
    let dir = tempdir().unwrap();
    for (format, ext) in [(Format::Srt, "srt"), (Format::Vtt, "vtt")] {
        let input = dir.path().join(format!("in.{ext}"));
        let output = dir.path().join(format!("out.{ext}"));
        let source = sample(format);
        fs::write(&input, &source).unwrap();
        for (by, threshold, expected) in [
            ("+250ms", None, [(1250, 4250), (3250, 5250)]),
            ("-2s", None, [(0, 2000), (1000, 3000)]),
            ("+1s", Some("00:00:03,000"), [(1000, 4000), (4000, 6000)]),
        ] {
            let mut command = bin();
            command.args([
                "shift",
                input.to_str().unwrap(),
                "--by",
                by,
                "-o",
                output.to_str().unwrap(),
            ]);
            if let Some(from) = threshold {
                command.args(["--from", from]);
            }
            command.assert().success();
            let transformed = parse(&fs::read_to_string(&output).unwrap()).unwrap();
            let original = parse(&source).unwrap();
            assert_eq!(transformed.len(), original.len());
            for (i, cue) in transformed.iter().enumerate() {
                assert_eq!(cue.text, original[i].text);
                assert_eq!(cue.index, original[i].index);
                assert_eq!((cue.start.ms, cue.end.ms), expected[i]);
            }
            assert_eq!(fs::read_to_string(&input).unwrap(), source);
        }
    }
}
