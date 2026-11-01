mod gh;
mod herdr;
mod text;

use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use gh::{CheckState, Fetched};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Terminal,
};
use std::{
    collections::HashSet,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    thread,
    time::{Duration, Instant},
};

const POLL_EVERY: Duration = Duration::from_secs(45);
const FOCUS_EVERY: Duration = Duration::from_millis(800);

enum Msg {
    /// Focus moved to a new cwd/branch; a fetch is about to start.
    Context { branch: Option<String> },
    Data(Fetched),
}

#[derive(Clone, Copy, PartialEq)]
enum Filter {
    All,
    Humans,
    Bots,
}

struct App {
    branch: Option<String>,
    data: Option<Fetched>,
    loading: bool,
    filter: Filter,
    checks_open: bool,
    /// Comment urls the user expanded; everything else renders folded.
    expanded: HashSet<String>,
    /// Fold key of the comment under the cursor (set by `draw`).
    sel_key: Option<String>,
    sel: usize,
    quit: bool,
}

/// Something the cursor can sit on, with the lines it renders to.
struct Item {
    lines: Vec<Line<'static>>,
    url: Option<String>,
    /// Set for foldable items (comments).
    fold_key: Option<String>,
}

fn spawn_worker(tx: mpsc::Sender<Msg>, refresh: Arc<AtomicBool>) {
    thread::spawn(move || {
        let mut last: Option<(String, String)> = None;
        let mut fetched_at = Instant::now() - POLL_EVERY;
        loop {
            let key = herdr::focused_cwd(last.is_none()).map(|cwd| {
                let b = gh::branch_of(&cwd).unwrap_or_default();
                (cwd, b)
            });
            // Focus on our own pane (None) keeps whatever we already show.
            if let Some(key) = key {
                let changed = last.as_ref() != Some(&key);
                let due = fetched_at.elapsed() >= POLL_EVERY || refresh.swap(false, Ordering::Relaxed);
                if changed || due {
                    if changed {
                        let b = (!key.1.is_empty()).then(|| key.1.clone());
                        if tx.send(Msg::Context { branch: b }).is_err() {
                            return;
                        }
                    }
                    let result = if key.1.is_empty() {
                        Fetched::NotARepo
                    } else {
                        gh::fetch(&key.0, &key.1)
                    };
                    fetched_at = Instant::now();
                    last = Some(key);
                    if tx.send(Msg::Data(result)).is_err() {
                        return;
                    }
                }
            }
            thread::sleep(FOCUS_EVERY);
        }
    });
}

fn style_dim() -> Style {
    Style::default().fg(Color::DarkGray)
}

fn build_header(app: &App, width: usize) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    match &app.data {
        Some(Fetched::Pr(pr)) => {
            let (label, color) = match (pr.state.as_str(), pr.is_draft) {
                ("OPEN", true) => ("DRAFT", Color::Gray),
                ("OPEN", false) => ("OPEN", Color::Green),
                ("MERGED", _) => ("MERGED", Color::Magenta),
                _ => ("CLOSED", Color::Red),
            };
            out.push(Line::from(vec![
                Span::styled(format!("#{} ", pr.number), Style::default().add_modifier(Modifier::BOLD)),
                Span::styled(format!(" {label} "), Style::default().fg(Color::Black).bg(color)),
                Span::styled(if app.loading { "  ⟳" } else { "" }, style_dim()),
            ]));
            for l in text::wrap(&pr.title, width) {
                out.push(Line::from(Span::styled(l, Style::default().add_modifier(Modifier::BOLD))));
            }
            out.push(Line::from(Span::styled(
                format!("→ {} · updated {}", pr.base_ref_name, text::ago(&pr.updated_at)),
                style_dim(),
            )));
        }
        Some(Fetched::NoPr) => out.push(Line::from(Span::styled(
            format!("No PR for {}", app.branch.as_deref().unwrap_or("this branch")),
            style_dim(),
        ))),
        Some(Fetched::NotARepo) => out.push(Line::from(Span::styled("Not in a git repository", style_dim()))),
        Some(Fetched::Error(e)) => out.push(Line::from(Span::styled(e.clone(), Style::default().fg(Color::Red)))),
        None => out.push(Line::from(Span::styled("Loading…", style_dim()))),
    }
    out
}

fn build_items(app: &App, width: usize) -> (Vec<Item>, Vec<Line<'static>>, Vec<Line<'static>>) {
    // (items, checks summary line group, comment tabs line group)
    let mut items = Vec::new();
    let Some(Fetched::Pr(pr)) = &app.data else {
        return (items, vec![], vec![]);
    };
    let passing = pr.status_check_rollup.iter().filter(|c| c.result() == CheckState::Pass).count();
    let failing = pr.status_check_rollup.iter().filter(|c| c.result() == CheckState::Fail).count();
    let pending = pr.status_check_rollup.iter().filter(|c| c.result() == CheckState::Pending).count();
    let mut summary = vec![Span::styled(if app.checks_open { "▾ " } else { "▸ " }, style_dim())];
    if failing > 0 {
        summary.push(Span::styled(format!("{failing} failing  "), Style::default().fg(Color::Red)));
    }
    if pending > 0 {
        summary.push(Span::styled(format!("{pending} pending  "), Style::default().fg(Color::Yellow)));
    }
    summary.push(Span::styled(format!("{passing} passing"), Style::default().fg(Color::Green)));
    let checks_line = vec![Line::from(summary)];

    if app.checks_open {
        for c in &pr.status_check_rollup {
            let (glyph, color, word) = match c.result() {
                CheckState::Pass => ("✓", Color::Green, "Successful"),
                CheckState::Fail => ("✗", Color::Red, "Failed"),
                CheckState::Pending => ("◌", Color::Yellow, "Running"),
                CheckState::Skipped => ("⊘", Color::DarkGray, "Skipped"),
            };
            let title = c.title();
            let room = width.saturating_sub(word.len() + 5);
            let shown: String = if title.chars().count() > room {
                title.chars().take(room.saturating_sub(1)).collect::<String>() + "…"
            } else {
                title
            };
            let pad = width.saturating_sub(shown.chars().count() + word.len() + 3);
            items.push(Item {
                lines: vec![Line::from(vec![
                    Span::styled(format!(" {glyph} "), Style::default().fg(color)),
                    Span::raw(shown),
                    Span::raw(" ".repeat(pad)),
                    Span::styled(word, style_dim()),
                ])],
                url: c.url().map(str::to_owned),
                fold_key: None,
            });
        }
    }

    let feed = pr.feed();
    let humans = feed.iter().filter(|c| !c.is_bot()).count();
    let bots = feed.len() - humans;
    let tab = |label: String, on: bool| {
        if on {
            Span::styled(format!(" {label} "), Style::default().add_modifier(Modifier::REVERSED))
        } else {
            Span::styled(format!(" {label} "), style_dim())
        }
    };
    let tabs = vec![Line::from(vec![
        Span::styled("COMMENTS ", Style::default().add_modifier(Modifier::BOLD)),
        tab(format!("1 All {}", feed.len()), app.filter == Filter::All),
        tab(format!("2 Humans {humans}"), app.filter == Filter::Humans),
        tab(format!("3 Bots {bots}"), app.filter == Filter::Bots),
    ])];

    for c in feed.iter().filter(|c| match app.filter {
        Filter::All => true,
        Filter::Humans => !c.is_bot(),
        Filter::Bots => c.is_bot(),
    }) {
        let mut lines = vec![Line::from(vec![
            Span::styled(c.author.login.clone(), Style::default().add_modifier(Modifier::BOLD)),
            Span::styled(format!("  {}", text::ago(&c.created_at)), style_dim()),
            if c.is_bot() { Span::styled("  BOT", Style::default().fg(Color::Cyan)) } else { Span::raw("") },
        ])];
        let body = text::wrap(&text::clean(&c.body), width.saturating_sub(2));
        let total = body.len();
        let fold_key = (!c.url.is_empty()).then(|| c.url.clone());
        let open = fold_key.as_ref().is_some_and(|k| app.expanded.contains(k));
        // Folded: first non-empty line as a preview. Expanded: whole body.
        let shown = if open { total } else { 1.min(total) };
        for l in body.into_iter().take(shown) {
            lines.push(Line::from(Span::raw(format!("  {l}"))));
        }
        if !open && total > shown {
            lines.push(Line::from(Span::styled(
                format!("  ▸ {} more lines (space expands)", total - shown),
                style_dim(),
            )));
        }
        lines.push(Line::default());
        items.push(Item { lines, url: (!c.url.is_empty()).then(|| c.url.clone()), fold_key });
    }
    (items, checks_line, tabs)
}

fn draw(f: &mut ratatui::Frame, app: &mut App) {
    let area = f.area();
    let width = area.width.saturating_sub(2) as usize;
    let header = build_header(app, width);
    let (items, checks_line, tabs) = build_items(app, width);
    app.sel = app.sel.min(items.len().saturating_sub(1));
    app.sel_key = items.get(app.sel).and_then(|i| i.fold_key.clone());

    let [head_a, body_a, foot_a] = Layout::vertical([
        Constraint::Length(header.len() as u16 + 1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(area);
    f.render_widget(Paragraph::new(header), head_a);

    // Body = checks summary, (check items), comments tabs, comment items.
    let n_checks = if app.checks_open {
        if let Some(Fetched::Pr(pr)) = &app.data { pr.status_check_rollup.len() } else { 0 }
    } else {
        0
    };
    let mut lines: Vec<Line> = Vec::new();
    let mut sel_line = 0usize;
    lines.extend(checks_line);
    for (i, item) in items.iter().enumerate() {
        if i == n_checks {
            lines.push(Line::default());
            lines.extend(tabs.clone());
            lines.push(Line::default());
        }
        if i == app.sel {
            sel_line = lines.len();
        }
        let last = item.lines.len().saturating_sub(1);
        for (j, l) in item.lines.iter().enumerate() {
            let mut l = l.clone();
            // Highlight every line of the selected item (not its trailing blank spacer).
            let selected = i == app.sel && !(j == last && last > 0 && l.spans.is_empty());
            if selected {
                let sel = Style::default().fg(Color::Black).bg(Color::Rgb(205, 210, 230));
                for s in l.spans.iter_mut() {
                    s.style = s.style.patch(sel).remove_modifier(Modifier::REVERSED | Modifier::DIM);
                }
                l.spans.insert(0, Span::styled("▌", Style::default().fg(Color::Blue).bg(Color::Rgb(205, 210, 230))));
                l = l.patch_style(Style::default().bg(Color::Rgb(205, 210, 230)));
            } else {
                l.spans.insert(0, Span::raw(" "));
            }
            lines.push(l);
        }
    }
    if items.len() <= n_checks {
        lines.push(Line::default());
        lines.extend(tabs.clone());
        lines.push(Line::from(Span::styled("  no comments", style_dim())));
    }
    let h = body_a.height as usize;
    let scroll = sel_line.saturating_sub(h / 3);
    f.render_widget(Paragraph::new(lines).scroll((scroll as u16, 0)), body_a);
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "j/k move · enter open · space fold · c checks · 1/2/3 filter · r refresh · q quit",
            style_dim(),
        ))),
        foot_a,
    );
    // Stash the open-able URL for the key handler.
    app_urls(app, &items);
}

thread_local! {
    static URLS: std::cell::RefCell<Vec<Option<String>>> = const { std::cell::RefCell::new(Vec::new()) };
}
fn app_urls(_app: &App, items: &[Item]) {
    URLS.with(|u| *u.borrow_mut() = items.iter().map(|i| i.url.clone()).collect());
}

/// Headless check: `herdr-pr-pane --once <dir>` prints what the pane would show.
fn once(dir: &str) {
    let branch = gh::branch_of(dir).unwrap_or_default();
    println!("branch: {branch}");
    match gh::fetch(dir, &branch) {
        Fetched::Pr(pr) => {
            println!("#{} {} draft={} base={} updated={}", pr.number, pr.state, pr.is_draft, pr.base_ref_name, text::ago(&pr.updated_at));
            println!("title: {}", pr.title);
            let n = |s: CheckState| pr.status_check_rollup.iter().filter(|c| c.result() == s).count();
            println!("checks: pass={} fail={} pending={} skipped={}", n(CheckState::Pass), n(CheckState::Fail), n(CheckState::Pending), n(CheckState::Skipped));
            for c in pr.status_check_rollup.iter().take(8) {
                println!("  {:?} {} {:?}", c.result(), c.title(), c.url());
            }
            let feed = pr.feed();
            let humans = feed.iter().filter(|c| !c.is_bot()).count();
            println!("comments: all={} humans={} bots={}", feed.len(), humans, feed.len() - humans);
            if let Some(c) = feed.iter().find(|c| c.author.login.contains("coderabbit")) {
                println!("--- sample ({}):", c.author.login);
                for l in text::wrap(&text::clean(&c.body), 40).iter().take(14) {
                    println!("  {l}");
                }
            }
        }
        other => println!("{other:?}"),
    }
}

fn main() -> std::io::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--toggle") {
        herdr::toggle();
        return Ok(());
    }
    if args.get(1).map(String::as_str) == Some("--fit") {
        herdr::fit_width(herdr::target_width());
        return Ok(());
    }
    if args.get(1).map(String::as_str) == Some("--once") {
        once(args.get(2).map(String::as_str).unwrap_or("."));
        return Ok(());
    }
    let (tx, rx) = mpsc::channel();
    let refresh = Arc::new(AtomicBool::new(false));
    spawn_worker(tx, refresh.clone());
    thread::spawn(|| herdr::fit_width(herdr::target_width()));

    enable_raw_mode()?;
    execute!(std::io::stdout(), EnterAlternateScreen)?;
    let mut term = Terminal::new(CrosstermBackend::new(std::io::stdout()))?;
    let mut app = App { branch: None, data: None, loading: true, filter: Filter::All, checks_open: false, expanded: HashSet::new(), sel_key: None, sel: 0, quit: false };

    while !app.quit {
        while let Ok(m) = rx.try_recv() {
            match m {
                Msg::Context { branch } => {
                    app.branch = branch;
                    app.loading = true;
                    app.sel = 0;
                    app.data = None;
                }
                Msg::Data(d) => {
                    app.data = Some(d);
                    app.loading = false;
                }
            }
        }
        term.draw(|f| draw(f, &mut app))?;
        if event::poll(Duration::from_millis(250))? {
            if let Event::Key(k) = event::read()? {
                if k.kind != KeyEventKind::Press {
                    continue;
                }
                match k.code {
                    KeyCode::Char('q') | KeyCode::Esc => app.quit = true,
                    KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => app.quit = true,
                    KeyCode::Char('j') | KeyCode::Down => app.sel += 1,
                    KeyCode::Char('k') | KeyCode::Up => app.sel = app.sel.saturating_sub(1),
                    KeyCode::Char('c') => {
                        app.checks_open = !app.checks_open;
                        app.sel = 0;
                    }
                    KeyCode::Char(' ') | KeyCode::Tab => {
                        if let Some(k) = app.sel_key.clone() {
                            if !app.expanded.remove(&k) {
                                app.expanded.insert(k);
                            }
                        }
                    }
                    KeyCode::Char('1') => { app.filter = Filter::All; app.sel = 0 }
                    KeyCode::Char('2') => { app.filter = Filter::Humans; app.sel = 0 }
                    KeyCode::Char('3') => { app.filter = Filter::Bots; app.sel = 0 }
                    KeyCode::Char('r') => {
                        refresh.store(true, Ordering::Relaxed);
                        app.loading = true;
                    }
                    KeyCode::Char('o') | KeyCode::Enter => {
                        let url = URLS.with(|u| u.borrow().get(app.sel).cloned().flatten());
                        let url = url.or_else(|| match &app.data {
                            Some(Fetched::Pr(pr)) => Some(pr.url.clone()),
                            _ => None,
                        });
                        if let Some(url) = url {
                            gh::open_url(&url);
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    disable_raw_mode()?;
    execute!(std::io::stdout(), LeaveAlternateScreen)?;
    Ok(())
}
