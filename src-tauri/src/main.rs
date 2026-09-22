// Prevents an extra console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(|a| a == "--cli").unwrap_or(false) {
        // Headless mode for scripts and for testing without the window:
        //   addonforge --cli installs | scan | check | update <key> | install <catalog-id> | catalog
        std::process::exit(addonforge_lib::cli(&args[1..]));
    }
    addonforge_lib::run()
}
