//! ratatui + ratatui-image picker used by `mr find` / `cf find`.
//!
//! Each row is 2 cells tall, with a 4×2 icon on the left and the
//! 2-line title/summary text on the right — same visual language as
//! the `search` command's inline output, but with a live selection
//! cursor and typed-filter narrowing.
//!
//! We hand-roll the list rendering instead of using ratatui's `List`
//! widget: `List` puts styled text in `ListItem`s, but there's no
//! supported way to embed a `StatefulImage` per item. Rolling our own
//! visible-window + per-row layout is ~40 lines and gets us the icons
//! we want.

use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow, bail};
use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use image::DynamicImage;
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::Line,
    widgets::Paragraph,
};
use ratatui_image::{StatefulImage, picker::Picker, protocol::StatefulProtocol};

use super::hit_view::{HitRow, format_downloads, truncate};

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

/// Interactive picker for `mr find` / `cf find`. Returns the chosen
/// `HitRow` — callers pull `.slug` off it to build the follow-up `Add`.
/// Errors when the user cancels (Esc / Ctrl-C).
///
/// Runs inside `spawn_blocking` because ratatui's event loop is
/// synchronous and probing the terminal for protocol capabilities uses
/// blocking stdio. The tokio runtime keeps ticking around it.
pub async fn pick_hit_tui(
    rows: Vec<HitRow>,
    icons: Vec<Option<PathBuf>>,
    prompt: &'static str,
) -> Result<HitRow> {
    let picked = tokio::task::spawn_blocking(move || run_picker(rows, icons, prompt)).await??;
    Ok(picked)
}

fn run_picker(
    rows: Vec<HitRow>,
    icons: Vec<Option<PathBuf>>,
    prompt: &'static str,
) -> Result<HitRow> {
    let mut terminal = ratatui::try_init()?;
    let result = run_event_loop(&mut terminal, rows, icons, prompt);
    // Always restore, even on error, so a panicky terminal doesn't leave
    // the user with a broken shell.
    ratatui::try_restore()?;
    result
}

fn run_event_loop(
    terminal: &mut ratatui::DefaultTerminal,
    rows: Vec<HitRow>,
    icons: Vec<Option<PathBuf>>,
    prompt: &'static str,
) -> Result<HitRow> {
    // Probe the terminal for graphics protocol + font-size. Falls back
    // to halfblocks silently on any failure — we still get a picker,
    // just with a pixelated icon.
    let picker = Picker::from_query_stdio().unwrap_or_else(|_| Picker::halfblocks());

    // Pre-decode each row's icon into a StatefulProtocol. Rows with no
    // icon (or a decode failure) get `None` and render an empty gutter.
    // Decoding is one-shot; the widget handles resize/encode on area
    // change from there.
    let mut protocols: Vec<Option<StatefulProtocol>> = icons
        .into_iter()
        .map(|opt| opt.and_then(|path| decode_protocol(&picker, &path)))
        .collect();

    let labels: Vec<(String, String)> = rows.iter().map(render_label_lines).collect();
    let mut state = PickerState {
        rows,
        labels,
        filter: String::new(),
        filtered: Vec::new(),
        selected: 0,
        scroll: 0,
    };
    state.recompute_filter();

    loop {
        terminal.draw(|f| render(f, &mut state, &mut protocols, prompt))?;

        let Event::Key(key) = crossterm::event::read()? else {
            continue;
        };
        // Ignore key-up events on terminals that emit them (Windows).
        if key.kind != KeyEventKind::Press && key.kind != KeyEventKind::Repeat {
            continue;
        }
        match key_action(&key) {
            Action::Cancel => bail!("cancelled"),
            Action::Confirm => {
                let row_i = *state
                    .filtered
                    .get(state.selected)
                    .ok_or_else(|| anyhow!("no selection"))?;
                return Ok(state.rows.swap_remove(row_i));
            }
            Action::Up => {
                if state.selected > 0 {
                    state.selected -= 1;
                }
            }
            Action::Down => {
                if state.selected + 1 < state.filtered.len() {
                    state.selected += 1;
                }
            }
            Action::AppendFilter(c) => {
                state.filter.push(c);
                state.recompute_filter();
                state.selected = 0;
                state.scroll = 0;
            }
            Action::PopFilter => {
                state.filter.pop();
                state.recompute_filter();
                state.selected = 0;
                state.scroll = 0;
            }
            Action::Noop => {}
        }
    }
}

enum Action {
    Cancel,
    Confirm,
    Up,
    Down,
    AppendFilter(char),
    PopFilter,
    Noop,
}

fn key_action(key: &KeyEvent) -> Action {
    match (key.code, key.modifiers) {
        (KeyCode::Esc, _) => Action::Cancel,
        (KeyCode::Char('c'), KeyModifiers::CONTROL) => Action::Cancel,
        (KeyCode::Enter, _) => Action::Confirm,
        (KeyCode::Up, _) | (KeyCode::Char('p'), KeyModifiers::CONTROL) => Action::Up,
        (KeyCode::Down, _) | (KeyCode::Char('n'), KeyModifiers::CONTROL) => Action::Down,
        (KeyCode::Backspace, _) => Action::PopFilter,
        (KeyCode::Char(c), m)
            if !m.contains(KeyModifiers::CONTROL) && !m.contains(KeyModifiers::ALT) =>
        {
            Action::AppendFilter(c)
        }
        _ => Action::Noop,
    }
}

struct PickerState {
    rows: Vec<HitRow>,
    /// Pre-rendered (header, summary) per row. Summary is `""` when the
    /// mod has none. Kept as two strings so we can style them
    /// differently — header at full brightness, summary dimmed.
    labels: Vec<(String, String)>,
    filter: String,
    /// Indices into `rows` that match the current filter, in order.
    filtered: Vec<usize>,
    /// Index into `filtered` (NOT `rows`) of the currently highlighted
    /// entry. Reset to 0 whenever the filter changes.
    selected: usize,
    /// First visible entry in `filtered` — scroll window start.
    scroll: usize,
}

impl PickerState {
    fn recompute_filter(&mut self) {
        let needle = self.filter.to_lowercase();
        self.filtered = self
            .labels
            .iter()
            .enumerate()
            .filter(|(_, (h, s))| {
                needle.is_empty()
                    || h.to_lowercase().contains(&needle)
                    || s.to_lowercase().contains(&needle)
            })
            .map(|(i, _)| i)
            .collect();
    }

    /// Keep the selection within the visible window by adjusting `scroll`.
    fn adjust_scroll(&mut self, visible_rows: usize) {
        if visible_rows == 0 {
            return;
        }
        if self.selected < self.scroll {
            self.scroll = self.selected;
        } else if self.selected >= self.scroll + visible_rows {
            self.scroll = self.selected + 1 - visible_rows;
        }
    }
}

fn render(
    f: &mut Frame<'_>,
    state: &mut PickerState,
    protocols: &mut [Option<StatefulProtocol>],
    prompt: &'static str,
) {
    // Rows: filter (1), body (fill), help (1).
    let [filter_area, body_area, help_area] = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .areas(f.area());

    f.render_widget(
        Paragraph::new(format!("{prompt} › {}", state.filter)),
        filter_area,
    );

    // Body: fixed-height rows, `ROW_HEIGHT` tall each.
    let visible_rows = (body_area.height / ROW_HEIGHT) as usize;
    state.adjust_scroll(visible_rows);

    for (screen_i, &row_i) in state
        .filtered
        .iter()
        .enumerate()
        .skip(state.scroll)
        .take(visible_rows)
    {
        let row_y = body_area.y + ((screen_i - state.scroll) as u16) * ROW_HEIGHT;
        let row_area = Rect {
            x: body_area.x,
            y: row_y,
            width: body_area.width,
            height: ROW_HEIGHT,
        };
        let is_selected = screen_i == state.selected;
        render_row(f, row_area, &state.labels[row_i], protocols[row_i].as_mut(), is_selected);
    }

    let help = Paragraph::new(
        "↑↓ navigate · type to filter · enter to add · esc to cancel",
    )
    .style(Style::default().add_modifier(Modifier::DIM));
    f.render_widget(help, help_area);
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
