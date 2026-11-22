use serde::{Deserialize, Serialize};
use std::{cell::Cell, fs, path::Path};
use tauri::{App, LogicalSize, WebviewWindow, WebviewWindowBuilder, WindowEvent};

#[cfg(target_os = "linux")]
fn client_size(window: &WebviewWindow) -> Option<LogicalSize<u32>> {
    use gtk::prelude::GtkWindowExt;
    // Wayland configure events include client-side decorations. GTK's size
    // uses the same logical client area as its initial set_default_size call.
    let (width, height) = window.gtk_window().ok()?.size();
    Some(LogicalSize::new(
        width.try_into().ok()?,
        height.try_into().ok()?,
    ))
}

#[cfg(not(target_os = "linux"))]
fn client_size(window: &WebviewWindow) -> Option<LogicalSize<u32>> {
    Some(
        window
            .inner_size()
            .ok()?
            .to_logical(window.scale_factor().ok()?),
    )
}

#[derive(Clone, Copy, Deserialize, Serialize)]
struct Geometry {
    width: u32,
    height: u32,
    maximized: bool,
}

impl Geometry {
    fn observe(&mut self, window: &WebviewWindow, min_width: f64, min_height: f64) {
        if !matches!(window.is_minimized(), Ok(false))
            || !matches!(window.is_fullscreen(), Ok(false))
        {
            return;
        }
        let Ok(maximized) = window.is_maximized() else {
            return;
        };
        self.maximized = maximized;
        if !maximized {
            if let Some(size) = client_size(window) {
                if f64::from(size.width) >= min_width && f64::from(size.height) >= min_height {
                    self.width = size.width;
                    self.height = size.height;
                }
            }
        }
    }

    fn save(&self, path: &Path) -> std::io::Result<()> {
        let mut temp = tempfile::NamedTempFile::new_in(path.parent().unwrap())?;
        serde_json::to_writer(&mut temp, self)?;
        temp.persist(path)?;
        Ok(())
    }
}

pub fn open(app: &App, dir: &Path) -> tauri::Result<()> {
    let mut config = app.config().app.windows[0].clone();
    let path = dir.join("window.json");
    let saved = fs::read(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Geometry>(&bytes).ok());
    let min_width = config.min_width.unwrap_or(1.0);
    let min_height = config.min_height.unwrap_or(1.0);
    if let Some(saved) =
        saved.filter(|s| f64::from(s.width) >= min_width && f64::from(s.height) >= min_height)
    {
        // Keep saved sizes in logical pixels across monitor and scaling changes.
        let monitor = app
            .primary_monitor()
            .ok()
            .flatten()
            .or_else(|| app.available_monitors().ok()?.into_iter().next());
        config.width = f64::from(saved.width.min(u16::MAX.into()));
        config.height = f64::from(saved.height.min(u16::MAX.into()));
        if let Some(m) = monitor {
            let bounds = m.work_area().size.to_logical::<f64>(m.scale_factor());
            if bounds.width > 0.0 && bounds.height > 0.0 {
                config.width = config.width.min(bounds.width.max(min_width));
                config.height = config.height.min(bounds.height.max(min_height));
            }
        }
        config.maximized = saved.maximized;
    }

    let builder = WebviewWindowBuilder::from_config(app, &config)?;
    #[cfg(target_os = "linux")]
    let builder = if config.maximized {
        // Tao normally maximizes after the first configure event on Linux.
        // Ask GTK before mapping, so the first frame is already maximized.
        builder.visible(false).maximized(false)
    } else {
        builder
    };
    let window = builder.build()?;
    #[cfg(target_os = "linux")]
    if config.maximized {
        use gtk::prelude::{GtkWindowExt, WidgetExt};
        if let Ok(native) = window.gtk_window() {
            native.maximize();
            native.show_all();
        } else {
            window.show()?;
        }
    }
    let state = Cell::new(Geometry {
        width: config.width.round() as u32,
        height: config.height.round() as u32,
        maximized: config.maximized,
    });
    let main = window.clone();
    window.on_window_event(move |event| {
        if !matches!(
            event,
            WindowEvent::Resized(_) | WindowEvent::CloseRequested { .. }
        ) {
            return;
        }
        let mut geometry = state.get();
        geometry.observe(&main, min_width, min_height);
        state.set(geometry);
        if matches!(event, WindowEvent::CloseRequested { .. }) && geometry.save(&path).is_err() {
            eprintln!("Could not save the window size.");
        }
    });
    Ok(())
}
