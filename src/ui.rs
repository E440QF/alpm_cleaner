use crate::app::{App, Focus, Mode, human_size};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap},
    Frame,
};

fn badge(pkg: &crate::db::Pkg) -> String {
    let mut b = String::new();
    b.push(if pkg.as_dep { 'D' } else { 'E' });
    if pkg.foreign {
        b.push('+'); // foreign/AUR
    }
    b
}

pub fn draw(f: &mut Frame, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(8),
            Constraint::Length(3),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(f.area());

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
        .split(chunks[0]);

    draw_list(f, app, cols[0]);
    draw_preview(f, app, cols[1]);
    draw_info(f, app, chunks[1]);
    draw_command(f, app, chunks[2]);
    draw_keybinds(f, chunks[3]);
    draw_prompt(f, app, chunks[4]);

    match app.mode {
        Mode::Confirm => draw_confirm(f, app),
        Mode::Help => draw_help(f),
        _ => {}
    }
}

fn draw_list(f: &mut Frame, app: &App, area: Rect) {
    let visible = app.visible();
    let height = area.height.saturating_sub(2) as usize;
    let start = app.cursor.saturating_sub(height.saturating_sub(1) / 2).min(visible.len().saturating_sub(height.min(visible.len())));
    let end = (start + height).min(visible.len());

    let items: Vec<ListItem> = visible[start..end]
        .iter()
        .enumerate()
        .map(|(k, &pi)| {
            let p = &app.pkgs[pi];
            let i = start + k;
            let mark = if app.checked.contains(&p.name) { "[x]" } else { "[ ]" };
            let cursor = if i == app.cursor && app.focus == Focus::List { ">" } else { " " };
            let style = if app.checked.contains(&p.name) {
                Style::default().fg(Color::Green)
            } else {
                Style::default()
            };
            let line = Line::from(vec![
                Span::raw(format!("{cursor}{mark} ")),
                Span::styled(p.name.clone(), style.add_modifier(Modifier::BOLD)),
                Span::raw(format!(" {} ", p.version)),
                Span::styled(format!("[{}]", badge(p)), Style::default().fg(Color::DarkGray)),
                Span::raw(format!(" {}", human_size(p.size))),
            ]);
            ListItem::new(line)
        })
        .collect();

    let title = if app.filter.is_empty() {
        format!(
            " Installed [{}|{}|{}] ({}/{}) ",
            app.sort.label(),
            app.source.label(),
            app.reason.label(),
            visible.len(),
            app.pkgs.len()
        )
    } else {
        format!(
            " Installed [{}|{}|{}] [find: {}] ({}/{}) ",
            app.sort.label(),
            app.source.label(),
            app.reason.label(),
            app.filter,
            visible.len(),
            app.pkgs.len()
        )
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(if app.focus == Focus::List {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default()
        });
    f.render_widget(List::new(items).block(block), area);
}

fn draw_preview(f: &mut Frame, app: &mut App, area: Rect) {
    let mode = if app.cascade { "-Rnsc" } else { "-Rns" };
    let stale = if app.preview_pending { " …" } else { "" };
    let selected = app.preview.selected(&app.checked);
    let pulled = app.preview.pulled(&app.checked);
    let n_entries = selected.len() + pulled.len();
    let cursor_at = if app.focus == Focus::Preview && n_entries > 0 {
        format!(" [{}/{}]", app.preview_cursor + 1, n_entries)
    } else {
        String::new()
    };
    let title = format!(
        " Would remove {mode}{stale}{cursor_at} ({}: {} pkgs) ",
        human_size(app.total_removal_size()),
        app.preview.removed.len()
    );

    // Line index of each removal entry, in cursor order (checked, then pulled).
    let mut entry_lines: Vec<usize> = Vec::with_capacity(n_entries);
    let hl = |idx: usize| app.focus == Focus::Preview && idx == app.preview_cursor;

    let mut lines: Vec<Line> = Vec::new();
    if app.checked.is_empty() {
        lines.push(Line::from(Span::styled(
            "check packages with <space> — preview appears here",
            Style::default().fg(Color::DarkGray),
        )));
    }
    let mut entry_idx = 0;
    for n in &selected {
        entry_lines.push(lines.len());
        let mut style = Style::default().fg(Color::Red).add_modifier(Modifier::BOLD);
        if hl(entry_idx) {
            style = style.add_modifier(Modifier::REVERSED);
        }
        lines.push(Line::from(vec![
            Span::styled("+ ", style),
            Span::styled(n.to_string(), style),
        ]));
        entry_idx += 1;
    }
    for n in &pulled {
        entry_lines.push(lines.len());
        let mut style = Style::default().fg(Color::Yellow);
        if hl(entry_idx) {
            style = style.add_modifier(Modifier::REVERSED);
        }
        lines.push(Line::from(vec![
            Span::styled("↳ ", style),
            Span::styled(n.to_string(), style),
        ]));
        entry_idx += 1;
    }
    if !app.preview.breakages.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "BLOCKED without -c (required by):",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        )));
        for (q, r) in app.preview.breakages.iter().take(50) {
            lines.push(Line::from(vec![
                Span::raw("! "),
                Span::raw(q.clone()),
                Span::styled(format!(" needs {r}"), Style::default().fg(Color::DarkGray)),
            ]));
        }
        if app.preview.breakages.len() > 50 {
            lines.push(Line::from(Span::styled(
                format!("… and {} more", app.preview.breakages.len() - 50),
                Style::default().fg(Color::DarkGray),
            )));
        }
    }
    if app.preview.hold_pkg {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "HoldPkg in target list — pacman refuses --print; toggle cascade off or uncheck it.",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        )));
    }
    if let Some(err) = &app.preview.error {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            err.to_string(),
            Style::default().fg(Color::Red),
        )));
    }
    if !app.preview.missing.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!("not installed: {}", app.preview.missing.join(", ")),
            Style::default().fg(Color::DarkGray),
        )));
    }

    let height = area.height.saturating_sub(2) as usize;
    // Keep the preview cursor visible; the stored offset only ever derives
    // from the (clamped) cursor, so overscroll can't bank up phantom presses.
    if height == 0 {
        app.preview_scroll = 0;
    } else if let Some(&line) = entry_lines.get(app.preview_cursor) {
        if line < app.preview_scroll {
            app.preview_scroll = line;
        } else if line >= app.preview_scroll + height {
            app.preview_scroll = line + 1 - height;
        }
    }
    let max_scroll = lines.len().saturating_sub(height);
    app.preview_scroll = app.preview_scroll.min(max_scroll);
    let scroll = app.preview_scroll as u16;
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(if app.focus == Focus::Preview {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default()
        });
    f.render_widget(
        Paragraph::new(lines)
            .block(block)
            .scroll((scroll, 0))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn draw_info(f: &mut Frame, app: &App, area: Rect) {
    let w = area.width.saturating_sub(2) as usize;
    let mut lines: Vec<Line> = Vec::new();
    match app.hovered() {
        None => lines.push(Line::from(Span::styled(
            "no package under cursor",
            Style::default().fg(Color::DarkGray),
        ))),
        Some(p) => {
            lines.push(Line::from(vec![
                Span::styled(
                    p.name.clone(),
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::raw(format!(" {}  {}", p.version, human_size(p.size))),
            ]));
            lines.push(Line::from(Span::styled(
                fit(&p.desc, w),
                Style::default().fg(Color::DarkGray),
            )));
            let date = p
                .install_date
                .and_then(|t| chrono::DateTime::from_timestamp(t, 0))
                .map(|d| d.format("%Y-%m-%d").to_string())
                .unwrap_or_else(|| "?".to_string());
            let mut meta = format!(
                "installed {date} · {} · {}",
                if p.as_dep { "dep" } else { "explicit" },
                if p.foreign { "AUR/foreign" } else { "repo" },
            );
            if !p.groups.is_empty() {
                meta.push_str(&format!(" · groups: {}", p.groups.join(",")));
            }
            if !p.licenses.is_empty() {
                meta.push_str(&format!(" · license: {}", p.licenses.join(",")));
            }
            if !p.provides.is_empty() {
                meta.push_str(&format!(" · provides: {}", p.provides.join(",")));
            }
            if !p.url.is_empty() {
                meta.push_str(&format!(" · {}", p.url));
            }
            lines.push(Line::from(fit(&meta, w)));
            lines.push(Line::from(join_fit("depends", &p.depends, w)));
            lines.push(Line::from(join_fit("optional", &p.optdepends, w)));
            let req: Vec<String> = crate::db::required_by(&app.pkgs, p)
                .iter()
                .map(|s| s.to_string())
                .collect();
            lines.push(Line::from(join_fit("required by", &req, w)));
        }
    }
    f.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Package info "),
        ),
        area,
    );
}

/// Truncate to `w` chars, marking with `…` when cut.
fn fit(s: &str, w: usize) -> String {
    if s.chars().count() <= w {
        return s.to_string();
    }
    if w == 0 {
        return String::new();
    }
    let mut out: String = s.chars().take(w.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// `label (n): a, b, c +k more`, fitted to `w` chars (overflow clipped).
fn join_fit<T: AsRef<str>>(label: &str, items: &[T], w: usize) -> String {
    if items.is_empty() {
        return format!("{label}: none");
    }
    let mut s = format!("{label} ({}): ", items.len());
    let mut shown = 0;
    for item in items {
        let add = if shown == 0 {
            item.as_ref().to_string()
        } else {
            format!(", {}", item.as_ref())
        };
        if s.chars().count() + add.chars().count() > w {
            break;
        }
        s.push_str(&add);
        shown += 1;
    }
    if shown < items.len() {
        s.push_str(&format!(" +{} more", items.len() - shown));
    }
    s
}

fn draw_command(f: &mut Frame, app: &App, area: Rect) {
    let p = Paragraph::new(Line::from(vec![
        Span::styled("$ ", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
        Span::raw(app.command_preview()),
    ]))
    .block(Block::default().borders(Borders::ALL).title(" Command "));
    f.render_widget(p, area);
}

fn draw_keybinds(f: &mut Frame, area: Rect) {
    let w = area.width as usize;
    let keys = "spc toggle · s sort · o source · e expl/dep · / search · c cascade · Tab panel · a apply · ? help · q quit";
    f.render_widget(
        Paragraph::new(Span::styled(fit(keys, w), Style::default().fg(Color::DarkGray))),
        area,
    );
}

fn draw_prompt(f: &mut Frame, app: &App, area: Rect) {
    let msg = if app.mode == Mode::Filter {
        format!(":{}▌", app.filter)
    } else if !app.status.is_empty() {
        app.status.clone()
    } else {
        app.state_summary()
    };
    f.render_widget(Paragraph::new(msg), area);
}

fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let x = area.x + area.width.saturating_sub(w) / 2;
    let y = area.y + area.height.saturating_sub(h) / 2;
    Rect { x, y, width: w.min(area.width), height: h.min(area.height) }
}

fn draw_confirm(f: &mut Frame, app: &App) {
    let area = centered(f.area(), 70, 12);
    f.render_widget(Clear, area);
    let targets = app.apply_targets();
    let shown = targets.iter().take(5).cloned().collect::<Vec<_>>().join(" ");
    let more = if targets.len() > 5 {
        format!(" (+{} more)", targets.len() - 5)
    } else {
        String::new()
    };
    let text = vec![
        Line::from(Span::styled(
            "Apply removal?",
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(app.command_preview()),
        Line::from(format!("explicit: {shown}{more}")),
        Line::from(format!(
            "total removed: {} pkgs, {}",
            app.preview.removed.len(),
            human_size(app.total_removal_size())
        )),
        Line::from(""),
        Line::from(Span::styled(
            "[y] run   [n/Esc] cancel",
            Style::default().fg(Color::Yellow),
        )),
    ];
    f.render_widget(
        Paragraph::new(text).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Confirm ")
                .border_style(Style::default().fg(Color::Red)),
        ),
        area,
    );
}

fn draw_help(f: &mut Frame) {
    let area = centered(f.area(), 60, 16);
    f.render_widget(Clear, area);
    let text = vec![
        Line::from("j/k, ↑/↓ move   PgUp/PgDn jump   g/G top/bottom"),
        Line::from("space/x check/uncheck   / or : search   Esc clear/close"),
        Line::from("s sort name/size   o source all/repo/AUR   e all/explicit/dep"),
        Line::from("Tab switch panel (j/k moves its cursor; info follows focus)"),
        Line::from("c toggle cascade (-Rns ↔ -Rnsc)"),
        Line::from("a apply (confirm with y)   q quit"),
        Line::from(""),
        Line::from("Badges: E explicit, D dep, + foreign/AUR."),
        Line::from("+ red = you checked it, ↳ yellow = pulled in."),
        Line::from("! red = blocks -Rns, needs -c (toggle with c)."),
        Line::from(""),
        Line::from("Priv escalation: $ALPM_CLEANER_PRIV or $SUDO,"),
        Line::from("else sudo/doas/run0 autodetect."),
        Line::from(""),
        Line::from(Span::styled("any key closes", Style::default().fg(Color::DarkGray))),
    ];
    f.render_widget(
        Paragraph::new(text).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Help ")
                .border_style(Style::default().fg(Color::Cyan)),
        ),
        area,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Pkg;
    use crate::privilege::Privilege;
    use ratatui::{Terminal, backend::TestBackend};

    fn pkg(name: &str, desc: &str, depends: &[&str]) -> Pkg {
        Pkg {
            name: name.into(),
            version: "1.0-1".into(),
            desc: desc.into(),
            size: 1024,
            as_dep: false,
            foreign: false,
            url: String::new(),
            groups: vec![],
            licenses: vec!["MIT".into()],
            depends: depends.iter().map(|s| s.to_string()).collect(),
            optdepends: vec![],
            provides: vec![],
            install_date: Some(1788889647),
        }
    }

    fn app() -> App {
        let pkgs = vec![
            pkg("app", "does things", &["lib"]),
            pkg("lib", "shared stuff", &[]),
        ];
        let idx = pkgs
            .iter()
            .enumerate()
            .map(|(i, p)| (p.name.clone(), i))
            .collect();
        let mut a = App::new(pkgs, idx, Privilege::None);
        // Bypass the background worker with a canned preview.
        a.preview_pending = false;
        a.preview.removed = vec!["app".into(), "lib".into()];
        a
    }

    fn render_text(a: &mut App, w: u16, h: u16) -> String {
        let backend = TestBackend::new(w, h);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| draw(f, a)).unwrap();
        term.backend().to_string()
    }

    #[test]
    fn info_follows_list_cursor() {
        let mut a = app();
        a.focus = Focus::List;
        a.cursor = 1;
        assert_eq!(a.hovered().unwrap().name, "lib");
        let out = render_text(&mut a, 100, 30);
        assert!(out.contains("shared stuff"), "info shows hovered desc");
        assert!(out.contains("required by (1): app"), "info shows required-by");
    }

    #[test]
    fn info_follows_preview_cursor() {
        let mut a = app();
        a.checked.insert("app".into());
        a.focus = Focus::Preview;
        a.preview_cursor = 1;
        assert_eq!(a.hovered().unwrap().name, "lib");
        let out = render_text(&mut a, 100, 30);
        assert!(out.contains("shared stuff"));
        assert!(out.contains("[2/2]"), "title shows preview cursor position");
    }

    #[test]
    fn fit_truncates() {
        assert_eq!(fit("hello", 10), "hello");
        assert_eq!(fit("hello world", 5), "hell…");
    }

    #[test]
    fn join_fit_caps_output() {
        let items = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        assert_eq!(join_fit("x", &items, 100), "x (3): a, b, c");
        assert_eq!(join_fit("x", &[] as &[String], 100), "x: none");
        let s = join_fit("x", &items, 12);
        assert!(s.contains("+"), "overflow summarized, got: {s}");
        assert!(s.chars().count() <= 20, "roughly bounded, got: {s}");
    }
}
