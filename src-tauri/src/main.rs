// Prevents an extra console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(|a| a == "--cli").unwrap_or(false) {
        // Headless mode for scripts and for testing without the window:
        //   addonforge --cli installs | scan | check | update <key> | install <catalog-id> | ...
        std::process::exit(addonforge_lib::cli(&args[1..]));
    }
    // Launched by a self-update: tidy up the old exe in the background.
    if args.first().map(|a| a == "--replaced").unwrap_or(false) {
        // Tell the app it was just updated, so the "what changed" note shows once.
        std::env::set_var("ADDONFORGE_JUST_UPDATED", "1");
        // The old exe is removed only after this copy's window is up (see setup in lib.rs).
        if let Some(old) = args.get(1) {
            addonforge_lib::note_replace(std::path::PathBuf::from(old), args.get(2).map(std::path::PathBuf::from));
        }
    }
    addonforge_lib::run()
}
