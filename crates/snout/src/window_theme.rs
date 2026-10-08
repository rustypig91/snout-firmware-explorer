use ashpd::desktop::settings::{ColorScheme, Settings};
use eframe::egui;

// Read the desktop preference once without blocking the GUI on D-Bus.
pub(super) fn apply_startup_theme(ctx: egui::Context) {
    if let Err(error) = std::thread::Builder::new()
        .name("window-theme".into())
        .spawn(move || {
            if let Err(error) = futures_lite::future::block_on(read_theme(ctx)) {
                eprintln!("Could not read the desktop window theme: {error}");
            }
        })
    {
        eprintln!("Could not start the desktop window theme check: {error}");
    }
}

async fn read_theme(ctx: egui::Context) -> ashpd::Result<()> {
    let settings = Settings::new().await?;
    apply_theme(&ctx, settings.color_scheme().await?);
    Ok(())
}

fn apply_theme(ctx: &egui::Context, scheme: ColorScheme) {
    let theme = match scheme {
        ColorScheme::PreferDark => egui::SystemTheme::Dark,
        ColorScheme::PreferLight | ColorScheme::NoPreference => egui::SystemTheme::Light,
    };
    ctx.send_viewport_cmd_to(
        egui::ViewportId::ROOT,
        egui::ViewportCommand::SetTheme(theme),
    );
    ctx.request_repaint();
}
