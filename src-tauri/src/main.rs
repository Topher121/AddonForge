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
        if let Some(old) = args.get(1).cloned() {
            std::thread::spawn(move || addonforge_lib::finish_replace(std::path::Path::new(&old)));
        }
    }
    addonforge_lib::run()
}
