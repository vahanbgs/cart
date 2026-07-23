//! Progress-bar styling. One template + one span factory so every
//! phase (mods, assets, java, libraries, forge deps) renders with the
//! same shape.
//!
//! Bars are attached to `tracing` spans and rendered by
//! `IndicatifLayer` (wired up in `main.rs`). Call sites create a span,
//! `.enter()` it, and call `pb_inc(1)` on it inside each parallel
//! future; the layer draws the bar at the bottom of the terminal and
//! suspends it around `tracing::info!`/etc. output.
//!
//! Span *name* has to be a literal for `tracing::info_span!`, so the
//! visible phase label is carried in `{msg}` (set via
//! `pb_set_message`) instead of `{span_name}`.
//!
//! Public re-exports let callers reach for `progress::IndicatifSpanExt`
//! without pulling in the `tracing-indicatif` name directly.

use indicatif::ProgressStyle;
use tracing::Span;
pub use tracing_indicatif::span_ext::IndicatifSpanExt;

fn style() -> ProgressStyle {
    ProgressStyle::with_template("{msg:>12.cyan.bold} [{bar:24.cyan/blue}] {pos:>4}/{len:<4}")
        .expect("valid progress template")
        .progress_chars("=> ")
}

/// Return a tracing span pre-configured as an `indicatif` bar with the
/// shared cart style. `label` is the right-aligned phase name; `len`
/// is the total item count. The caller enters the span (`.enter()` or
/// `.instrument()`) and increments it via `IndicatifSpanExt::pb_inc`.
pub fn bar(label: &str, len: u64) -> Span {
    let span = tracing::info_span!("cart_progress");
    span.pb_set_style(&style());
    span.pb_set_length(len);
    span.pb_set_message(label);
    span
}
