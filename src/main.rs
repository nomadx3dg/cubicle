use cubicle::cli::{parse_args, Parsed, USAGE};
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // A bare `cubicle` opens the menu with the saved settings.
    let bare = args.is_empty();
    match parse_args(args) {
        Ok(Parsed::Help) => {
            print!("{USAGE}");
            ExitCode::SUCCESS
        }
        Ok(Parsed::Version) => {
            println!("cubicle {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Ok(Parsed::Run(cfg)) => {
            let result = if !cfg.once && (bare || cfg.menu) {
                cubicle::app::run_menu(&cfg, bare)
            } else {
                cubicle::app::run(&cfg)
            };
            match result {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("cubicle: {e}");
                    ExitCode::from(1)
                }
            }
        }
        Err(e) => {
            eprintln!("cubicle: {e}");
            ExitCode::from(2)
        }
    }
}
