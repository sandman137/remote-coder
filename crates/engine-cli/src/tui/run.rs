//! Terminal runner: raw mode + alternate screen + poll loop. All state logic
//! lives in `App`; this file only pumps events and repaints.

use std::io;
use std::time::{Duration, Instant};

use anyhow::Result;
use ratatui::crossterm::event::{self, Event, KeyEventKind};
use ratatui::crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::crossterm::ExecutableCommand;
use ratatui::prelude::CrosstermBackend;
use ratatui::Terminal;

use super::{ui, App, POLL_INTERVAL_MS};

fn restore_terminal() {
    let _ = disable_raw_mode();
    let _ = io::stdout().execute(LeaveAlternateScreen);
}

pub async fn run(mut app: App) -> Result<()> {
    enable_raw_mode()?;
    io::stdout().execute(EnterAlternateScreen)?;
    // Never leave the user's terminal raw, even on panic.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore_terminal();
        default_hook(info);
    }));

    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;

    app.refresh_panes().await;
    let mut last_tick = Instant::now();

    let result: Result<()> = loop {
        if let Err(e) = terminal.draw(|f| ui::draw(f, &mut app)) {
            break Err(e.into());
        }

        // Drain pending terminal events without blocking the poll cadence.
        while event::poll(Duration::ZERO)? {
            match event::read()? {
                Event::Key(key) if key.kind != KeyEventKind::Release => {
                    app.handle_key(key).await;
                }
                _ => {}
            }
        }
        if app.should_quit {
            break Ok(());
        }

        if app.dirty || last_tick.elapsed() >= Duration::from_millis(POLL_INTERVAL_MS) {
            app.tick().await;
            last_tick = Instant::now();
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    };

    restore_terminal();
    result
}
