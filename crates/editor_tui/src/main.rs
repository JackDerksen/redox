use editor_core::*;

use minui::prelude::*;

fn main() -> minui::Result<()> {
    let mut app = App::new(())?;

    // Application handler for event loops and rendering updates
    app.run(
        |_state, event| {
            // Closure for handling input and updates.
            match event {
                Event::KeyWithModifiers(k) if matches!(k.key, KeyKind::Char('q')) => false,
                Event::Character('q') => false,
                _ => true,
            }
        },
        |_state, window| {
            // Closure for rendering the application state.
            let label = Label::new("Welcome to Redox (WIP). Press 'q' to quit")
                .with_alignment(Alignment::Center);

            label.draw(window)?;

            window.flush()?;

            Ok(()) // Drawing succeeded
        },
    )?;

    Ok(())
}
