use minui::prelude::*;

fn main() -> minui::Result<()> {
    let mut app = App::new(())?;

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
            let label = Label::new("Press 'q' to quit").with_alignment(Alignment::Center);

            label.draw(window)?;

            window.flush()?;

            Ok(()) // Drawing succeeded
        },
    )?;

    Ok(())
}
