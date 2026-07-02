# srt-tools

A small, fast, dependency-light Rust CLI for manipulating subtitle files
(SubRip `.srt` and WebVTT `.vtt`). Shift timing, convert between formats, merge
multiple files, fix broken/scrambled subtitles, correct framerate drift, and
report file statistics.

The SRT/VTT parser is hand-written (no regex, no heavy subtitle crate) and is
tolerant of real-world quirks: CRLF or LF line endings, a UTF-8 BOM, blank lines
between cues, VTT headers / `NOTE` blocks / cue identifiers, VTT cue settings on
the timing line, and `.`-vs-`,` millisecond separators.

All timestamps are stored as integer milliseconds, so shift/scale arithmetic is
exact and lossless.

## Install

```sh
cargo build --release
# binary at target/release/srt-tools
```

## Usage

Every subcommand reads from a file argument or, if omitted (or given as `-`),
from **stdin**, and writes to `-o <file>` or, if omitted, to **stdout**. Output
format for `convert` is inferred from the `-o` extension (or forced with `--to`).

### shift — move all timestamps by a signed duration

```sh
srt-tools shift in.srt --by +2.5s -o out.srt      # later by 2.5 seconds
srt-tools shift in.srt --by -1.2s -o out.srt      # earlier by 1.2 seconds
srt-tools shift in.srt --by 00:00:02,500 -o out.srt
srt-tools shift in.srt --by +1s --from 00:10:00,000 -o out.srt  # only cues at/after 10:00
```

`--by` accepts: `+2.5s` / `-1.2s` / `3s` (seconds), `1200ms` (milliseconds),
`1m` (minutes), a bare number like `2.5` (seconds), or a signed timestamp
`HH:MM:SS,mmm`. Times never go below zero.

### convert — SRT <-> VTT

```sh
srt-tools convert in.srt -o out.vtt    # SRT -> VTT (inferred from .vtt)
srt-tools convert in.vtt -o out.srt    # VTT -> SRT
srt-tools convert in.srt --to vtt      # force format, write to stdout
```

### merge — concatenate several files, renumber, keep times

```sh
srt-tools merge part1.srt part2.srt part3.srt -o full.srt
srt-tools merge a.srt b.srt --offset 1m -o full.srt   # +1 min gap before each later file
```

### fix — clean up a broken subtitle file

```sh
srt-tools fix messy.srt -o clean.srt
```

Renumbers sequentially, sorts by start time, clamps overlaps (a cue can't run
past the next cue's start), ensures `end >= start`, drops empty cues, and
normalizes line endings.

### stats — inspect a file without changing it

```sh
srt-tools stats in.srt
cat in.srt | srt-tools stats
```

Prints the cue count, the first start and last end, the total span, the summed
on-screen time, and the coverage (on-screen time as a percentage of the span):

```
cues:      3
first:     00:00:01,000
last:      00:00:12,500
span:      00:00:11,500
on-screen: 00:00:07,250
coverage:  63.0%
```

### scale — correct framerate drift (linear)

```sh
srt-tools scale in.srt --factor 1.0010010 -o out.srt   # 23.976 -> 24 fps
srt-tools scale in.srt --factor 0.95904   -o out.srt   # 25 -> 23.976 fps
```

Multiplies every timestamp by `--factor` (rounded to the nearest millisecond).

## Piping

```sh
cat in.srt | srt-tools shift --by +1s | srt-tools convert --to vtt > out.vtt
```

## Library

The logic lives in a small library (`srt_tools`) so it is unit-testable and
reusable:

```rust
use srt_tools::{parse, shift, to_srt};

let mut cues = parse(&text)?;
shift(&mut cues, 2500, None);   // +2.5 s
let out = to_srt(&cues);
```

Key items: `Timestamp` (ms-precision, `to_srt`/`to_vtt`), `Cue`,
`parse`, `to_srt`, `to_vtt`, `shift`, `scale`, `merge`, `fix`, `renumber`,
`stats` (with the `Stats` summary type).

## Testing

```sh
cargo test
```

Unit tests cover parsing, exact shift/scale results, the fix reorder/renumber
pass, and an SRT->VTT->SRT round-trip. Integration tests drive the real binary
for every subcommand and assert that malformed input exits non-zero with a clear
error.

## License

MIT — see [LICENSE](LICENSE).
