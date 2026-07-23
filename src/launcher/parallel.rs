//! Bounded-concurrency helpers for the launcher's cold-cache fetch
//! phase. All HTTP fetches share one `reqwest::Client` (and thus one
//! connection pool), so the total in-flight budget is a process-wide
//! property — expose a single knob rather than per-site numbers that
//! could drift.

use futures::{Future, StreamExt, TryStreamExt};

/// Max concurrent HTTP fetches per bounded-stream loop. Tuned to fill
/// a fast link without tripping CDN rate limits on Mojang / Modrinth /
/// CurseForge, and comfortably below reqwest's default per-host pool
/// cap so multiple loops sharing this client can coexist.
const FETCH_CONCURRENCY: usize = 8;

/// Run `f` on each item with up to [`FETCH_CONCURRENCY`] futures in
/// flight; futures may complete out of order. Use when downstream
/// consumers don't care about iteration order (e.g. cache warm-ups).
pub async fn run<I, F, Fut>(items: I, f: F) -> anyhow::Result<()>
where
    I: IntoIterator,
    F: FnMut(I::Item) -> Fut,
    Fut: Future<Output = anyhow::Result<()>>,
{
    futures::stream::iter(items)
        .map(f)
        .buffer_unordered(FETCH_CONCURRENCY)
        .try_collect::<()>()
        .await
}

/// Run `f` on each item with up to [`FETCH_CONCURRENCY`] futures in
/// flight, yielding results in the original iteration order. Use when
/// a downstream sequential pass depends on that order (e.g. classpath
/// dedup where insertion order determines position and same-key
/// replacement).
pub async fn collect<I, F, Fut, T>(items: I, f: F) -> anyhow::Result<Vec<T>>
where
    I: IntoIterator,
    F: FnMut(I::Item) -> Fut,
    Fut: Future<Output = anyhow::Result<T>>,
{
    futures::stream::iter(items)
        .map(f)
        .buffered(FETCH_CONCURRENCY)
        .try_collect()
        .await
}
