use alpm_cleaner::app::{App, Focus, Mode};
use alpm_cleaner::{db, privilege, ui};
use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io::Stdout;
use std::process::Command;

fn main() -> Result<()> {
    let (pkgs, idx) = db::load_db()?;
    let priv_ = privilege::detect();
    let mut app = App::new(pkgs, idx, priv_);
    app.status = format!(
        "loaded {} packages — privilege: {}",
        app.pkgs.len(),
        app.priv_.display()
    );

    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut term = Terminal::new(backend)?;

    let res = run(&mut term, &mut app);

    disable_raw_mode()?;
    execute!(term.backend_mut(), LeaveAlternateScreen)?;
    term.show_cursor()?;
    if let Err(e) = res {
        eprintln!("error: {e:#}");
    }
    Ok(())
}

fn run(term: &mut Terminal<CrosstermBackend<Stdout>>, app: &mut App) -> Result<()> {
    loop {
        app.poll_preview();
        term.draw(|f| ui::draw(f, &mut *app))?;

        if !event::poll(std::time::Duration::from_millis(150))? {
            continue;
        }
        let Event::Key(k) = event::read()? else {
            continue;
        };

        // Help dismisses on any key.
        if app.mode == Mode::Help {
            app.mode = Mode::Normal;
            continue;
        }

        // Filter input mode.
        if app.mode == Mode::Filter {
            match k.code {
                KeyCode::Esc | KeyCode::Enter => app.mode = Mode::Normal,
                KeyCode::Backspace => {
                    app.filter.pop();
                    app.cursor = 0;
                }
                KeyCode::Char(c) if !k.modifiers.contains(KeyModifiers::CONTROL) => {
                    app.filter.push(c);
                    app.cursor = 0;
                }
                _ => {}
            }
            continue;
        }

        // Confirm mode.
        if app.mode == Mode::Confirm {
            match k.code {
                KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                    app.mode = Mode::Normal;
                    apply(term, app)?;
                }
                KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Char('q') => {
                    app.mode = Mode::Normal;
                    app.status = "apply cancelled".to_string();
                }
                _ => {}
            }
            continue;
        }

        // Normal mode. Stale status messages clear on any keypress so the
        // bottom line falls back to the filter/sort state summary.
        app.status.clear();
        match k.code {
            KeyCode::Char('q') => return Ok(()),
            KeyCode::Char('?') => app.mode = Mode::Help,
            KeyCode::Tab => {
                app.focus = match app.focus {
                    Focus::List => Focus::Preview,
                    Focus::Preview => Focus::List,
                };
            }
            KeyCode::Char('/') | KeyCode::Char(':') => {
                app.mode = Mode::Filter;
            }
            KeyCode::Char('s') => {
                app.sort.toggle();
                app.clamp_cursor();
            }
            KeyCode::Char('o') => {
                app.source.cycle();
                app.clamp_cursor();
            }
            KeyCode::Char('e') => {
                app.reason.cycle();
                app.clamp_cursor();
            }
            KeyCode::Esc => {
                if !app.filter.is_empty() {
                    app.filter.clear();
                    app.cursor = 0;
                }
            }
            KeyCode::Char('c') => {
                app.cascade = !app.cascade;
                app.recompute();
                app.status = if app.cascade {
                    "mode: -Rnsc (cascade)".to_string()
                } else {
                    "mode: -Rns".to_string()
                };
            }
            KeyCode::Char('a') => {
                if app.apply_targets().is_empty() {
                    app.status = "nothing checked — nothing to apply".to_string();
                } else {
                    app.mode = Mode::Confirm;
                }
            }
            KeyCode::Char(' ') | KeyCode::Char('x') | KeyCode::Enter
                if app.focus == Focus::List =>
            {
                app.toggle_cursor();
            }
            _ => {}
        }

        // Movement. On the preview pane j/k moves its cursor (the view
        // follows); the info panel shows the hovered package on either pane.
        match k.code {
            KeyCode::Up | KeyCode::Char('k') => {
                if app.focus == Focus::Preview {
                    app.move_preview_cursor(-1);
                } else {
                    app.move_cursor(-1);
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if app.focus == Focus::Preview {
                    app.move_preview_cursor(1);
                } else {
                    app.move_cursor(1);
                }
            }
            KeyCode::PageUp => {
                if app.focus == Focus::Preview {
                    app.move_preview_cursor(-10);
                } else {
                    app.move_cursor(-10);
                }
            }
            KeyCode::PageDown => {
                if app.focus == Focus::Preview {
                    app.move_preview_cursor(10);
                } else {
                    app.move_cursor(10);
                }
            }
            KeyCode::Home | KeyCode::Char('g') => {
                if app.focus == Focus::Preview {
                    app.move_preview_cursor(isize::MIN);
                } else {
                    app.cursor = 0;
                }
            }
            KeyCode::End | KeyCode::Char('G') => {
                if app.focus == Focus::Preview {
                    app.move_preview_cursor(isize::MAX);
                } else {
                    let n = app.visible().len();
                    app.cursor = n.saturating_sub(1);
                }
            }
            _ => {}
        }
    }
}

/// Suspend the TUI, run `pacman -Rns[c]`, then resume and reload the db.
fn apply(term: &mut Terminal<CrosstermBackend<Stdout>>, app: &mut App) -> Result<()> {
    let targets = app.apply_targets();
    if targets.is_empty() {
        return Ok(());
    }
    let mut argv = app.priv_.prefix_args();
    argv.extend(privilege::pacman_argv(app.cascade, &targets));

    disable_raw_mode()?;
    execute!(term.backend_mut(), LeaveAlternateScreen)?;

    println!("$ {}", argv.join(" "));
    let status = Command::new(&argv[0]).args(&argv[1..]).status();
    match status {
        Ok(s) => {
            println!("\n-- exit: {s} --");
            if !s.success() && !app.cascade && !app.preview.breakages.is_empty() {
                println!(
                    "hint: {} dependents block -Rns; press 'c' for -Rnsc cascade mode.",
                    app.preview.breakages.len()
                );
            }
        }
        Err(e) => println!("\n-- failed to run: {e} --"),
    }
    println!("press Enter to return…");
    let mut line = String::new();
    let _ = std::io::stdin().read_line(&mut line);

    execute!(term.backend_mut(), EnterAlternateScreen)?;
    enable_raw_mode()?;
    term.clear()?;

    match db::load_db() {
        Ok((pkgs, idx)) => {
            app.reload(pkgs, idx);
            app.status = format!("reloaded {} packages", app.pkgs.len());
        }
        Err(e) => app.status = format!("reload failed: {e:#}"),
    }
    Ok(())
}
