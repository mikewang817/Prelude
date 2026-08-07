# Contributing

Thanks for looking. Bug reports, especially ones with a screenshot of the
list rendering wrong, are genuinely the most valuable thing you can send.

## Developer Certificate of Origin

This project uses the [Developer Certificate of Origin](https://developercertificate.org/)
rather than a Contributor License Agreement. Sign off your commits:

```sh
git commit -s -m "your message"
```

That adds a `Signed-off-by:` line, which certifies you wrote the patch or
otherwise have the right to submit it under this project's license. No
paperwork, no copyright assignment — you keep the copyright to your work.

**What this means for both of us.** Contributions land under Apache-2.0 and
stay there. The project cannot relicense your code or move it into a
proprietary component without asking you first. If that ever becomes
necessary, contributors will be asked individually.

## Getting set up

```sh
cargo build --release
cargo test
cargo clippy --release      # expected to be warning-free
```

`prelude doctor` reports what it can see on your machine, and
`prelude bench` measures candidate-gathering.

## What matters in this codebase

**Latency is the product.** A launcher that takes 250ms to appear feels
broken. `prelude bench` must stay under 40ms, and startup matters just as
much as gathering — fzf re-invokes the binary on *every keystroke* to decide
what to show, so anything added to process startup is paid hundreds of times
per session. Be sceptical of new dependencies for this reason.

**Sources must degrade to nothing.** A source that shells out and fails, or
finds nothing, returns an empty list. It never blocks, never panics, never
prints. Anything slower than ~25ms belongs behind the cache tier rather than
in the hot path.

**Never execute without the user seeing it.** Enter inserts, it does not run.
This is not a preference, it is the safety property that makes the whole
thing bindable to a key you press all day. New actions default to inserting;
running is opt-in and labelled.

**Never index or transmit credentials.** `secrets.rs` filters history and
clipboard entries. If you add a source that reads user data, route it
through there.

## Rendering, which is fiddlier than it looks

Three traps that have each already caused a bug:

- **Column widths must be constants, not measurements.** The per-keystroke
  helper renders in a *separate process*. If both sides measure their own
  widths they drift apart and every computed row lands in the wrong column.
- **Display width is not character count.** CJK characters are two columns
  wide, and East Asian *Ambiguous* characters (`·` `—` `“”` `→`) are one or
  two depending on the terminal. Use `width::dwidth`, never `.len()` or
  `.chars().count()`.
- **fzf matches against displayed text.** A row computed *from* the query can
  therefore never fuzzy-match it. That is why `is_special()` exists and why
  it must stay pure pattern-matching — it runs on every keystroke, so it must
  not evaluate anything.

## Platform

macOS only at present. Ports, processes, apps, clipboard, system commands and
translation all use macOS-specific interfaces. Linux support would be welcome;
it means adding an implementation behind each of those sources rather than
changing the core.
