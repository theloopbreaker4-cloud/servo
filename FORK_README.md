# Aurora Servo Fork

This is a personal fork of [servo/servo](https://github.com/servo/servo)
maintained as the rendering engine for the
[Aurora Browser](https://github.com/theloopbreaker4-cloud/AuroraBrowserRust)
project.

## What's different from upstream

The fork carries a small set of additions on top of upstream Servo. Each
sits in its own commit with a clear message describing the why:

- **CSS counters** — pre-order DOM walk + `counter()` / `counters()`
  resolution in `components/layout/lists.rs` and `counters.rs`. Upstream
  has the data structures but not the resolution pass.
- **`backdrop-filter` rendering** — emits a `PushBackdropFilter` display
  item before the stacking context so `filter()` / `blur()` / etc. apply
  to whatever is painted underneath. Pairs with a small stylo patch.
- **Native scrollbar** — Aurora-themed thin scrollbar painted from the
  scroll tree, with mouse drag and click-track-to-page-scroll. Lives in
  `components/paint/scrollbar.rs`.
- **Body text selection (in progress)** — `position_for_point` for
  pixel→character resolution (`components/layout/display_list/`),
  document-wide `body_selection` field, and mouse-drag handlers. Same-
  node selection works; cross-node is a follow-up.
- **A handful of warning fixes** kept in `chore:` commits.

## Tracking upstream

The fork uses the standard `origin` (this fork) + `upstream`
(servo/servo) remote setup. New upstream work is merged in periodically
via `git pull upstream main` so the fork's additions ride on top of
current Servo, not a frozen snapshot.

## How this fork is maintained

This fork is my experiment in browser-building on top of Servo. I started
it, I drive the direction, I test, and I make the architectural decisions
about what to add.

Most of the Rust code is written by Claude AI to my specifications and
under my supervision. I'm still learning Rust — the AI accelerates
implementation, I provide direction, review, and verification.

I'm writing this openly because I think transparency matters more than
the impression I make.

## License

This fork is distributed under the same MPL 2.0 license as upstream
Servo. Modifications are © their respective contributors and licensed
under MPL 2.0; see individual file headers and commit history.
