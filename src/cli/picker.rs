//! ratatui + ratatui-image live-search picker used by `mr find` / `cf find`.
//!
//! Each row is 2 cells tall, with a 4×2 icon on the left and the
//! 2-line title/summary text on the right — same visual language as
//! the `search` command's inline output. The query line at the top is
//! the *actual* search: keystrokes edit it, and after a short debounce
//! the query hits the Modrinth / CurseForge API, filters out mods
//! already in `cart.toml`, and refreshes the result set.
//!
//! We hand-roll the list rendering instead of using ratatui's `List`
//! widget: `List` puts styled text in `ListItem`s, but there's no
//! supported way to embed a `StatefulImage` per item. Rolling our own
//! visible-window + per-row layout is ~40 lines and gets us the icons
//! we want.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use crossterm::cursor::MoveTo;
use crossterm::event::{Event, EventStream, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::style::Print;
use futures::StreamExt;
use image::DynamicImage;
use ratatui::{
    Frame, TerminalOptions, Viewport,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::Line,
    widgets::Paragraph,
};
use ratatui_image::{StatefulImage, picker::Picker, protocol::StatefulProtocol};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::{Instant, sleep_until};

use super::hit_view::{HitRow, format_downloads, icon_urls, prefetch_icons, truncate};
use super::icon_cache::IconCache;

const SUMMARY_MAX_CHARS: usize = 80;
/// Left gutter for the selection bar (`▎` + one padding cell). Blank on
/// unselected rows, so it doubles as consistent left padding.
const CURSOR_CELLS_WIDE: u16 = 2;
/// Icon gutter width, matching `hit_view`'s search-view constant so
/// the picker and `search` output look consistent.
const ICON_CELLS_WIDE: u16 = 4;
/// One-cell padding between icon and text.
const ICON_TEXT_GAP: u16 = 1;
/// Total vertical space per row: title line + summary line. Also the
/// icon's height, so the icon fills the row exactly.
const ROW_HEIGHT: u16 = 2;
/// Fixed inline viewport height: query (1) + body (12) + help (1).
/// The result set changes over time so we can't grow / shrink the
/// viewport with it; reserving the full 14 rows up front keeps the
/// picker's on-screen footprint stable across queries.
const INLINE_HEIGHT: u16 = 14;
/// How long to wait after the last keystroke before firing a fetch.
/// Long enough to absorb a fluent typist between-key gap; short
/// enough that a deliberate pause fires visibly quickly.
const SEARCH_DEBOUNCE: Duration = Duration::from_millis(250);
/// How many distinct queries to keep in the session cache before
/// evicting the oldest. Bounded so backspace-then-retype paths are
/// instant without letting a lot of unique queries balloon memory.
const QUERY_CACHE_MAX: usize = 64;

/// A search backend for the interactive picker: Modrinth or CurseForge.
/// Impls capture per-session state (HTTP client, MC version, loader,
/// already-installed slugs / project ids) and return backend-blind
/// `HitRow`s. The installed-filter lives inside the impl so the picker
/// stays backend-blind.
pub trait Backend: Send + Sync + 'static {
    fn search(
        &self,
        query: String,
        limit: u32,
    ) -> impl std::future::Future<Output = Result<Vec<HitRow>>> + Send;
}

/// Live-search picker for `mr find` / `cf find`. Returns the chosen
/// `HitRow` — callers pull `.slug` off it to build the follow-up `Add`.
/// Returns `Ok(None)` when the user cancels (Esc / Ctrl-C) or when the
/// event stream terminates.
///
/// `initial_query` seeds the input (empty is fine — opens the picker
/// ready to type). `limit` caps how many hits each search asks for.
pub async fn pick_hit_interactive<B: Backend>(
    backend: B,
    icon_cache: IconCache,
    initial_query: String,
    limit: u32,
    prompt: &'static str,
) -> Result<Option<HitRow>> {
    let mut terminal = ratatui::try_init_with_options(TerminalOptions {
        viewport: Viewport::Inline(INLINE_HEIGHT),
    })?;
    let backend = Arc::new(backend);
    let icon_cache = Arc::new(icon_cache);
    let result = run_event_loop(
        &mut terminal,
        backend,
        icon_cache,
        initial_query,
        limit,
        prompt,
    )
    .await;
    // Collapse the inline viewport so the shell prompt (or the follow-up
    // `Add`'s tracing output) lands right where the picker started, not
    // below the reserved rows. `Terminal::clear()` alone only *blanks*
    // the reserved rows and restores the cursor below them — we park the
    // cursor at the viewport top-left and use the CSI `M` (Delete Line)
    // escape to physically remove those rows, shifting anything below up.
    //
    // `try_restore` runs first because its `LeaveAlternateScreen`
    // (`\x1b[?1049l`) is treated as a DECRC on xterm-family terminals —
    // restoring a "saved cursor" we never set. If we positioned the
    // cursor before that, it'd get yanked to a stale row and Add's
    // tracing lines would appear far below the viewport.
    let viewport = terminal.get_frame().area();
    ratatui::try_restore()?;
    let _ = execute!(
        std::io::stdout(),
        MoveTo(viewport.x, viewport.y),
        Print(format!("\x1b[{}M", viewport.height)),
    );
    result
}

#[derive(Clone)]
enum Status {
    Idle,
    Searching,
    Error(String),
}

struct PickerState {
    query: String,
    rows: Vec<HitRow>,
    /// Pre-rendered (header, summary) per row. Rebuilt whenever `rows`
    /// changes so `render` stays alloc-light on the hot path.
    labels: Vec<(String, String)>,
    /// Index into `rows` of the currently highlighted entry.
    selected: usize,
    /// First visible entry in `rows` — scroll window start.
    scroll: usize,
    status: Status,
    /// Per-session cache of `lowercased-query` → `(rows, icon paths)`.
    /// Backspace-then-retype avoids a network round-trip.
    cache: HashMap<String, (Vec<HitRow>, Vec<Option<PathBuf>>)>,
    /// FIFO insertion order for `cache`, capped at `QUERY_CACHE_MAX`.
    cache_order: VecDeque<String>,
}

impl PickerState {
    fn new(initial_query: String) -> Self {
        Self {
            query: initial_query,
            rows: Vec::new(),
            labels: Vec::new(),
            selected: 0,
            scroll: 0,
            status: Status::Idle,
            cache: HashMap::new(),
            cache_order: VecDeque::new(),
        }
    }

    fn adjust_scroll(&mut self, visible_rows: usize) {
        if visible_rows == 0 || self.rows.is_empty() {
            self.scroll = 0;
            return;
        }
        if self.selected < self.scroll {
            self.scroll = self.selected;
        } else if self.selected >= self.scroll + visible_rows {
            self.scroll = self.selected + 1 - visible_rows;
        }
    }

    fn set_rows(&mut self, rows: Vec<HitRow>) {
        self.labels = rows.iter().map(render_label_lines).collect();
        self.rows = rows;
        self.selected = 0;
        self.scroll = 0;
    }

    fn cache_insert(&mut self, query: String, rows: Vec<HitRow>, icons: Vec<Option<PathBuf>>) {
        if !self.cache.contains_key(&query) {
            self.cache_order.push_back(query.clone());
            if self.cache_order.len() > QUERY_CACHE_MAX
                && let Some(evict) = self.cache_order.pop_front()
            {
                self.cache.remove(&evict);
            }
        }
        self.cache.insert(query, (rows, icons));
    }
}

enum Msg {
    Fetched {
        query: String,
        rows: Vec<HitRow>,
        icons: Vec<Option<PathBuf>>,
    },
    Failed {
        query: String,
        err: String,
    },
}

async fn run_event_loop<B: Backend>(
    terminal: &mut ratatui::DefaultTerminal,
    backend: Arc<B>,
    icon_cache: Arc<IconCache>,
    initial_query: String,
    limit: u32,
    prompt: &'static str,
) -> Result<Option<HitRow>> {
    // Probe the terminal for graphics protocol + font-size. Falls back
    // to halfblocks silently on any failure — we still get a picker,
    // just with a pixelated icon. The probe uses blocking stdio, so
    // hop off the async thread for the one-shot call.
    let picker = tokio::task::spawn_blocking(|| {
        Picker::from_query_stdio().unwrap_or_else(|_| Picker::halfblocks())
    })
    .await
    .unwrap_or_else(|_| Picker::halfblocks());

    let mut state = PickerState::new(initial_query);
    let mut protocols: Vec<Option<StatefulProtocol>> = Vec::new();

    let mut events = EventStream::new();
    let (tx, mut rx) = mpsc::channel::<Msg>(4);
    let mut inflight: Option<JoinHandle<()>> = None;

    // Debounce timer. Starts already-elapsed but gated by
    // `debounce_active`, so it never fires spuriously. Reset via
    // `Sleep::reset` on each query change.
    let sleep = sleep_until(Instant::now());
    tokio::pin!(sleep);
    let mut debounce_active = false;

    // If we opened with a non-empty initial query, kick off the fetch
    // immediately (no debounce) so results appear as soon as the picker
    // paints.
    if !state.query.is_empty() {
        fire_fetch(
            &backend,
            &icon_cache,
            &state.query,
            limit,
            &tx,
            &mut inflight,
            &mut state.status,
        );
    }

    loop {
        terminal.draw(|f| render(f, &mut state, &mut protocols, prompt))?;

        tokio::select! {
            maybe_ev = events.next() => {
                let Some(ev_res) = maybe_ev else { return Ok(None); };
                let ev = ev_res?;
                let Event::Key(key) = ev else { continue; };
                if key.kind != KeyEventKind::Press && key.kind != KeyEventKind::Repeat {
                    continue;
                }
                match key_action(&key) {
                    Action::Cancel => return Ok(None),
                    Action::Confirm => {
                        if state.selected < state.rows.len() {
                            return Ok(Some(state.rows.swap_remove(state.selected)));
                        }
                    }
                    Action::Up => {
                        if state.selected > 0 {
                            state.selected -= 1;
                        }
                    }
                    Action::Down => {
                        if state.selected + 1 < state.rows.len() {
                            state.selected += 1;
                        }
                    }
                    Action::AppendChar(c) => {
                        state.query.push(c);
                        on_query_change(
                            &mut state,
                            &mut protocols,
                            &picker,
                            &mut inflight,
                            sleep.as_mut(),
                            &mut debounce_active,
                        );
                    }
                    Action::Backspace => {
                        state.query.pop();
                        on_query_change(
                            &mut state,
                            &mut protocols,
                            &picker,
                            &mut inflight,
                            sleep.as_mut(),
                            &mut debounce_active,
                        );
                    }
                    Action::Noop => {}
                }
            }

            _ = &mut sleep, if debounce_active => {
                debounce_active = false;
                if !state.query.is_empty() {
                    fire_fetch(
                        &backend,
                        &icon_cache,
                        &state.query,
                        limit,
                        &tx,
                        &mut inflight,
                        &mut state.status,
                    );
                }
            }

            Some(msg) = rx.recv() => {
                match msg {
                    Msg::Fetched { query, rows, icons } => {
                        state.cache_insert(query.clone(), rows.clone(), icons.clone());
                        if query == state.query {
                            protocols = build_protocols(&icons, &picker);
                            state.set_rows(rows);
                            state.status = Status::Idle;
                        }
                    }
                    Msg::Failed { query, err } => {
                        if query == state.query {
                            state.status = Status::Error(err);
                        }
                    }
                }
            }
        }
    }
}

/// Called after the user edits the query. Cancels any pending fetch,
/// serves from the session cache when possible, and otherwise arms the
/// debounce timer so a fresh fetch fires after the user pauses typing.
/// Stale rows stay visible until fresh ones arrive.
fn on_query_change(
    state: &mut PickerState,
    protocols: &mut Vec<Option<StatefulProtocol>>,
    picker: &Picker,
    inflight: &mut Option<JoinHandle<()>>,
    mut sleep: std::pin::Pin<&mut tokio::time::Sleep>,
    debounce_active: &mut bool,
) {
    if let Some(handle) = inflight.take() {
        handle.abort();
    }

    if state.query.is_empty() {
        state.set_rows(Vec::new());
        protocols.clear();
        state.status = Status::Idle;
        *debounce_active = false;
        return;
    }

    if let Some((rows, icons)) = state.cache.get(&state.query) {
        *protocols = build_protocols(icons, picker);
        state.set_rows(rows.clone());
        state.status = Status::Idle;
        *debounce_active = false;
        return;
    }

    sleep.as_mut().reset(Instant::now() + SEARCH_DEBOUNCE);
    *debounce_active = true;
}

/// Spawn the actual search + icon-prefetch task, replacing any prior
/// in-flight fetch. Result lands on `tx`; the receiver checks `query`
/// against the current input before applying so stale hits are ignored
/// (but still get cached for later reuse).
fn fire_fetch<B: Backend>(
    backend: &Arc<B>,
    icon_cache: &Arc<IconCache>,
    query: &str,
    limit: u32,
    tx: &mpsc::Sender<Msg>,
    inflight: &mut Option<JoinHandle<()>>,
    status: &mut Status,
) {
    if let Some(handle) = inflight.take() {
        handle.abort();
    }
    let b = backend.clone();
    let c = icon_cache.clone();
    let q = query.to_owned();
    let tx = tx.clone();
    *status = Status::Searching;
    *inflight = Some(tokio::spawn(async move {
        let msg = do_fetch(b, c, q, limit).await;
        let _ = tx.send(msg).await;
    }));
}

/// Standalone helper so the compiler can infer HRTB lifetimes cleanly
/// on the `prefetch_icons` closure — inlining this into `fire_fetch`'s
/// `tokio::spawn` state machine confuses rustc's HRTB inference.
async fn do_fetch<B: Backend>(
    backend: Arc<B>,
    icon_cache: Arc<IconCache>,
    query: String,
    limit: u32,
) -> Msg {
    match backend.search(query.clone(), limit).await {
        Ok(rows) => {
            let icons = prefetch_icons(icon_cache.as_ref(), icon_urls(&rows)).await;
            Msg::Fetched { query, rows, icons }
        }
        Err(e) => {
            let err = format!("{e:#}");
            Msg::Failed { query, err }
        }
    }
}

fn build_protocols(icons: &[Option<PathBuf>], picker: &Picker) -> Vec<Option<StatefulProtocol>> {
    icons
        .iter()
        .map(|opt| opt.as_ref().and_then(|p| decode_protocol(picker, p)))
        .collect()
}

enum Action {
    Cancel,
    Confirm,
    Up,
    Down,
    AppendChar(char),
    Backspace,
    Noop,
}

fn key_action(key: &KeyEvent) -> Action {
    match (key.code, key.modifiers) {
        (KeyCode::Esc, _) => Action::Cancel,
        (KeyCode::Char('c'), KeyModifiers::CONTROL) => Action::Cancel,
        (KeyCode::Enter, _) => Action::Confirm,
        (KeyCode::Up, _) | (KeyCode::Char('p'), KeyModifiers::CONTROL) => Action::Up,
        (KeyCode::Down, _) | (KeyCode::Char('n'), KeyModifiers::CONTROL) => Action::Down,
        (KeyCode::Backspace, _) => Action::Backspace,
        (KeyCode::Char(c), m)
            if !m.contains(KeyModifiers::CONTROL) && !m.contains(KeyModifiers::ALT) =>
        {
            Action::AppendChar(c)
        }
        _ => Action::Noop,
    }
}

fn render(
    f: &mut Frame<'_>,
    state: &mut PickerState,
    protocols: &mut [Option<StatefulProtocol>],
    prompt: &'static str,
) {
    // Rows: query (1), body (fill), help (1).
    let [query_area, body_area, help_area] = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .areas(f.area());

    f.render_widget(
        Paragraph::new(format!("{prompt} › {}", state.query)),
        query_area,
    );

    if state.rows.is_empty() {
        let hint = if state.query.is_empty() {
            "Type to search"
        } else {
            match &state.status {
                Status::Searching => "Searching…",
                Status::Error(_) => "no results",
                Status::Idle => "no results",
            }
        };
        f.render_widget(
            Paragraph::new(hint).style(Style::default().add_modifier(Modifier::DIM)),
            body_area,
        );
    } else {
        let visible_rows = (body_area.height / ROW_HEIGHT) as usize;
        state.adjust_scroll(visible_rows);

        for (screen_i, row_i) in (state.scroll..state.rows.len())
            .take(visible_rows)
            .enumerate()
        {
            let row_y = body_area.y + (screen_i as u16) * ROW_HEIGHT;
            let row_area = Rect {
                x: body_area.x,
                y: row_y,
                width: body_area.width,
                height: ROW_HEIGHT,
            };
            let is_selected = row_i == state.selected;
            render_row(
                f,
                row_area,
                &state.labels[row_i],
                protocols.get_mut(row_i).and_then(|o| o.as_mut()),
                is_selected,
            );
        }
    }

    let help_text = match &state.status {
        Status::Searching => "Searching…".to_owned(),
        Status::Error(err) => format!("error: {}", first_line(err)),
        Status::Idle => "↑↓ navigate · type to search · enter to add · esc to cancel".to_owned(),
    };
    let help = Paragraph::new(help_text).style(Style::default().add_modifier(Modifier::DIM));
    f.render_widget(help, help_area);
}

fn first_line(s: &str) -> &str {
    s.split_once('\n').map(|(a, _)| a).unwrap_or(s)
}

fn render_row(
    f: &mut Frame<'_>,
    row_area: Rect,
    label: &(String, String),
    protocol: Option<&mut StatefulProtocol>,
    is_selected: bool,
) {
    // Split: cursor gutter + icon gutter + gap + text.
    let [cursor_area, icon_area, _gap, text_area] = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(CURSOR_CELLS_WIDE),
            Constraint::Length(ICON_CELLS_WIDE),
            Constraint::Length(ICON_TEXT_GAP),
            Constraint::Min(1),
        ])
        .areas(row_area);

    if is_selected {
        // Full-height bar spanning both rows of the entry.
        let bar = Paragraph::new(vec![Line::from("▎"), Line::from("▎")])
            .style(Style::default().fg(Color::LightCyan));
        f.render_widget(bar, cursor_area);
    }

    if let Some(protocol) = protocol {
        f.render_stateful_widget(StatefulImage::default(), icon_area, protocol);
    }

    let (header, summary) = label;
    let header_line = if is_selected {
        Line::from(header.as_str()).style(Style::default().fg(Color::LightCyan))
    } else {
        Line::from(header.as_str())
    };
    let summary_line = if summary.is_empty() {
        Line::from("")
    } else {
        let truncated = truncate(summary, SUMMARY_MAX_CHARS);
        Line::from(format!("  {truncated}")).dim()
    };
    f.render_widget(Paragraph::new(vec![header_line, summary_line]), text_area);
}

fn decode_protocol(picker: &Picker, path: &Path) -> Option<StatefulProtocol> {
    // `IconCache` stores files without an extension, so
    // `ImageReader::open(path).decode()` alone fails with "no format" —
    // we have to sniff the magic bytes via `with_guessed_format`.
    let img: DynamicImage = match image::ImageReader::open(path)
        .and_then(|r| r.with_guessed_format())
        .map_err(anyhow::Error::from)
        .and_then(|r| r.decode().map_err(Into::into))
    {
        Ok(img) => img,
        Err(err) => {
            tracing::debug!("icon decode failed for {}: {err}", path.display());
            return None;
        }
    };
    Some(picker.new_resize_protocol(img))
}

/// Split the row label into (header, summary) so the picker can style
/// them independently. Mirrors the `search` command's format so users
/// see the same layout in both contexts.
fn render_label_lines(row: &HitRow) -> (String, String) {
    let header = format!(
        "{title} · {slug} · {downloads}",
        title = row.title,
        slug = row.slug,
        downloads = format_downloads(row.downloads),
    );
    (header, row.summary.clone())
}
