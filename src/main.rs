//! `srt-tools` — command-line front-end over the `srt_tools` library.
//!
//! All real logic lives in `src/lib.rs`; this file only parses arguments, does
//! I/O (file or stdin/stdout), and maps results to process exit codes.

use std::io::{Read, Write};
use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};
use clap::{Parser, Subcommand};

use srt_tools::{self as lib, Cue, Format, ParseError, Stats, Timestamp};

#[derive(Parser)]
#[command(
    name = "srt-tools",
    version,
    about = "Shift, convert, merge, fix, scale, and inspect SRT/VTT subtitle files.",
    long_about = None,
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Shift all timestamps by a signed duration (e.g. +2.5s, -1.2s, 00:00:02,500).
    Shift {
        /// Input file (omit or use '-' to read SRT/VTT from stdin).
        input: Option<String>,
        /// Amount to shift by: +2.5s, -1200ms, 1.5, or HH:MM:SS,mmm (may be negative).
        #[arg(long = "by", allow_hyphen_values = true)]
        by: String,
        /// Only shift cues that start at or after this timestamp.
        #[arg(long = "from")]
        from: Option<String>,
        /// Output file (omit to write to stdout). Format inferred from extension.
        #[arg(short = 'o', long = "output")]
        output: Option<String>,
    },

    /// Convert between SRT and VTT (format inferred from the -o extension).
    Convert {
        /// Input file (omit or '-' for stdin).
        input: Option<String>,
        /// Output file; extension picks the format (.vtt or .srt).
        #[arg(short = 'o', long = "output")]
        output: Option<String>,
        /// Force output format instead of inferring from -o ("srt" or "vtt").
        #[arg(long = "to")]
        to: Option<String>,
    },

    /// Concatenate multiple files, renumber, and keep times.
    Merge {
        /// Two or more input files, in order.
        #[arg(required = true, num_args = 1..)]
        inputs: Vec<String>,
        /// Output file (omit for stdout).
        #[arg(short = 'o', long = "output")]
        output: Option<String>,
        /// Cumulative time gap inserted before each successive file (e.g. 1m, 500ms).
        #[arg(long = "offset", allow_hyphen_values = true)]
        offset: Option<String>,
    },

    /// Renumber, sort by start time, clamp overlaps, drop empty cues.
    Fix {
        /// Input file (omit or '-' for stdin).
        input: Option<String>,
        /// Output file (omit for stdout).
        #[arg(short = 'o', long = "output")]
        output: Option<String>,
    },

    /// Report cue count, time span, on-screen duration, and coverage.
    Stats {
        /// Input file (omit or '-' for stdin).
        input: Option<String>,
    },

    /// Linearly scale all timestamps by a factor (framerate drift, e.g. 1.001).
    Scale {
        /// Input file (omit or '-' for stdin).
        input: Option<String>,
        /// Multiplicative factor (> 0), e.g. 1.0010010 for 23.976->24.
        #[arg(long = "factor")]
        factor: f64,
        /// Output file (omit for stdout).
        #[arg(short = 'o', long = "output")]
        output: Option<String>,
    },
}

fn main() {
    if let Err(err) = run() {
        // Print the whole error chain to stderr, then exit non-zero.
        eprintln!("error: {err:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Shift {
            input,
            by,
            from,
            output,
        } => {
            let delta =
                parse_duration(&by).with_context(|| format!("invalid --by duration {by:?}"))?;
            let from_ts = match from {
                Some(ref s) => Some(
                    Timestamp::parse(s).map_err(|e| anyhow!("invalid --from timestamp: {e}"))?,
                ),
                None => None,
            };
            let mut cues = read_cues(input.as_deref())?;
            validate_shift(&cues, delta, from_ts)?;
            lib::shift(&mut cues, delta, from_ts);
            write_cues(&cues, output.as_deref(), None)?;
        }

        Command::Convert { input, output, to } => {
            let cues = read_cues(input.as_deref())?;
            let format = resolve_format(to.as_deref(), output.as_deref())?;
            write_cues(&cues, output.as_deref(), Some(format))?;
        }

        Command::Merge {
            inputs,
            output,
            offset,
        } => {
            if inputs.len() < 2 {
                bail!(
                    "merge needs at least two input files (got {})",
                    inputs.len()
                );
            }
            let offset_ms = match offset {
                Some(ref s) => {
                    parse_duration(s).with_context(|| format!("invalid --offset {s:?}"))?
                }
                None => 0,
            };
            let mut lists: Vec<Vec<Cue>> = Vec::with_capacity(inputs.len());
            for path in &inputs {
                lists.push(read_cues(Some(path))?);
            }
            for (k, cues) in lists.iter().enumerate() {
                let delta = i64::try_from(k)
                    .ok()
                    .and_then(|index| offset_ms.checked_mul(index))
                    .ok_or_else(|| {
                        anyhow!("cumulative --offset exceeds the signed millisecond range")
                    })?;
                validate_shift(cues, delta, None)?;
            }
            let merged = lib::merge(&lists, offset_ms);
            write_cues(&merged, output.as_deref(), None)?;
        }

        Command::Fix { input, output } => {
            let cues = read_cues(input.as_deref())?;
            let fixed = lib::fix(cues);
            if fixed.is_empty() {
                bail!("after fix, no cues remained (input had only empty cues)");
            }
            write_cues(&fixed, output.as_deref(), None)?;
        }

        Command::Stats { input } => {
            // A cue-less file is not an error for `stats`: report zero cues
            // rather than exiting non-zero the way transforming commands do.
            let raw = read_raw(input.as_deref())?;
            let cues = match lib::parse(&raw) {
                Ok(cues) => cues,
                Err(ParseError::Empty) => Vec::new(),
                Err(e) => bail!("failed to parse subtitles: {e}"),
            };
            let report = format_stats(&lib::stats(&cues));
            // Write through locked stdout (not `print!`, which panics on EPIPE
            // when the reader goes away, e.g. `stats f | true`).
            let stdout = std::io::stdout();
            let mut h = stdout.lock();
            h.write_all(report.as_bytes())
                .context("writing stats report to stdout")?;
        }

        Command::Scale {
            input,
            factor,
            output,
        } => {
            if !(factor > 0.0) || !factor.is_finite() {
                bail!("--factor must be a positive, finite number (got {factor})");
            }
            let mut cues = read_cues(input.as_deref())?;
            lib::scale(&mut cues, factor);
            write_cues(&cues, output.as_deref(), None)?;
        }
    }
    Ok(())
}

/// Parse a signed duration into milliseconds. Accepts:
///   * `+2.5s` / `-1.2s` / `3s`          (seconds, fractional ok)
///   * `1200ms` / `-500ms`               (milliseconds)
///   * `2.5` / `-1`                      (bare number = seconds)
///   * `HH:MM:SS,mmm` / `MM:SS.mmm`      (signed timestamp form)
fn parse_duration(raw: &str) -> Result<i64> {
    let s = raw.trim();
    if s.is_empty() {
        bail!("empty duration");
    }

    // Timestamp form (contains a ':'). May carry a leading sign.
    if s.contains(':') {
        let (sign, body) = split_sign(s);
        let ts = Timestamp::parse(body).map_err(|e| anyhow!("{e}"))?;
        return Ok(sign * ts.ms);
    }

    let (num, unit_ms) = if let Some(num) = s.strip_suffix("ms") {
        (num.trim(), 1)
    } else if let Some(num) = s.strip_suffix('s') {
        (num.trim(), 1000)
    } else if let Some(num) = s.strip_suffix('m') {
        (num.trim(), 60_000)
    } else {
        (s, 1000) // bare number: seconds
    };

    // Keep whole-number boundaries exact; f64 rounds i64::MAX up to 2^63.
    let (_, digits) = split_sign(num);
    if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) {
        let value: i64 = num
            .parse()
            .context("duration exceeds the signed millisecond range")?;
        return value
            .checked_mul(unit_ms)
            .ok_or_else(|| anyhow!("duration exceeds the signed millisecond range"));
    }

    let value: f64 = num
        .parse()
        .with_context(|| format!("not a number: {num:?}"))?;
    let ms = (value * unit_ms as f64).round();
    if !ms.is_finite() {
        bail!("duration must be finite");
    }
    // Reject float boundaries: nearby out-of-range text can round onto them.
    // Exact whole-number durations above still support both i64 limits.
    if ms <= i64::MIN as f64 || ms >= -(i64::MIN as f64) {
        bail!("duration exceeds the signed millisecond range");
    }
    Ok(ms as i64)
}

/// Validate both endpoints of the same original-start-selected cues as `shift`.
/// Finish this check before transforming cues or opening an output destination.
fn validate_shift(cues: &[Cue], delta_ms: i64, from: Option<Timestamp>) -> Result<()> {
    for cue in cues {
        if from.is_some_and(|threshold| cue.start < threshold) {
            continue;
        }
        if cue.start.checked_shifted(delta_ms).is_none()
            || cue.end.checked_shifted(delta_ms).is_none()
        {
            bail!("shifted cue {} exceeds the timestamp range", cue.index);
        }
    }
    Ok(())
}

/// Render [`Stats`] as an aligned, human-readable report (trailing newline).
/// Durations reuse the SRT `HH:MM:SS,mmm` timestamp form.
fn format_stats(s: &Stats) -> String {
    let first = s
        .first_start
        .map(|t| t.to_srt())
        .unwrap_or_else(|| "-".to_string());
    let last = s
        .last_end
        .map(|t| t.to_srt())
        .unwrap_or_else(|| "-".to_string());
    let span = Timestamp::from_ms(s.span_ms).to_srt();
    let display = Timestamp::from_ms(s.display_ms).to_srt();
    format!(
        "cues:      {}\n\
         first:     {}\n\
         last:      {}\n\
         span:      {}\n\
         on-screen: {}\n\
         coverage:  {:.1}%\n",
        s.count,
        first,
        last,
        span,
        display,
        s.coverage() * 100.0,
    )
}

/// Split a leading '+' / '-' from a value, returning (+1|-1, rest).
fn split_sign(s: &str) -> (i64, &str) {
    if let Some(rest) = s.strip_prefix('-') {
        (-1, rest.trim_start())
    } else if let Some(rest) = s.strip_prefix('+') {
        (1, rest.trim_start())
    } else {
        (1, s)
    }
}

/// Decide the output format from an explicit `--to`, else the `-o` extension,
/// else default to SRT (used by `convert` only).
fn resolve_format(to: Option<&str>, output: Option<&str>) -> Result<Format> {
    if let Some(t) = to {
        return match t.to_ascii_lowercase().as_str() {
            "srt" => Ok(Format::Srt),
            "vtt" => Ok(Format::Vtt),
            other => bail!("unknown --to format {other:?} (expected srt or vtt)"),
        };
    }
    match output {
        Some(path) => Ok(Format::from_path(path)),
        None => {
            bail!("convert needs either --to <srt|vtt> or an -o file with a .srt/.vtt extension")
        }
    }
}

/// Read raw subtitle text from a file path, or from stdin when `path` is `None`
/// or `"-"`.
fn read_raw(path: Option<&str>) -> Result<String> {
    match path {
        Some("-") | None => {
            let mut buf = String::new();
            std::io::stdin()
                .read_to_string(&mut buf)
                .context("reading subtitles from stdin")?;
            Ok(buf)
        }
        Some(p) => {
            std::fs::read_to_string(p).with_context(|| format!("reading subtitle file {p:?}"))
        }
    }
}

/// Read and parse subtitle cues from a file path, or from stdin when `path` is
/// `None` or `"-"`.
fn read_cues(path: Option<&str>) -> Result<Vec<Cue>> {
    let raw = read_raw(path)?;
    lib::parse(&raw).map_err(|e| anyhow!("failed to parse subtitles: {e}"))
}

/// Write cues to a file (`-o`) or stdout. If `forced` is `None`, the format is
/// inferred from the output path's extension (stdout defaults to SRT).
fn write_cues(cues: &[Cue], output: Option<&str>, forced: Option<Format>) -> Result<()> {
    let format = forced.unwrap_or_else(|| match output {
        Some(p) => Format::from_path(p),
        None => Format::Srt,
    });
    let rendered = lib::to_format(cues, format);

    match output {
        Some(p) => {
            if let Some(parent) = Path::new(p).parent() {
                if !parent.as_os_str().is_empty() {
                    std::fs::create_dir_all(parent).ok();
                }
            }
            std::fs::write(p, rendered).with_context(|| format!("writing {p:?}"))?;
        }
        None => {
            let stdout = std::io::stdout();
            let mut h = stdout.lock();
            h.write_all(rendered.as_bytes())
                .context("writing to stdout")?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duration_units_signs_and_rounding_are_preserved() {
        for (raw, expected) in [
            ("+2.5s", 2500),
            ("-1.2s", -1200),
            ("1200ms", 1200),
            ("-0.5ms", -1),
            ("1m", 60_000),
            ("2.5", 2500),
            ("-00:00:02,500", -2500),
            ("1e-3s", 1),
        ] {
            assert_eq!(parse_duration(raw).unwrap(), expected, "{raw}");
        }
    }

    #[test]
    fn duration_integer_limits_are_exact_and_float_boundaries_are_rejected() {
        assert_eq!(parse_duration("9223372036854775807ms").unwrap(), i64::MAX);
        assert_eq!(parse_duration("-9223372036854775808ms").unwrap(), i64::MIN);
        assert_eq!(
            parse_duration("9.223372036854775e18ms").unwrap(),
            9_223_372_036_854_774_784
        );
        for raw in [
            "NaN",
            "inf",
            "-inf",
            "1e300m",
            "9223372036854775808ms",
            "-9223372036854775809ms",
            "-9223372036854775808.0ms",
            "-9223372036854775809.0ms",
            "-9.223372036854776e18ms",
            "9.223372036854776e18ms",
        ] {
            assert!(parse_duration(raw).is_err(), "{raw}");
        }
    }
}
