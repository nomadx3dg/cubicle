use cubicle::cli::{parse_args, Parsed, USAGE};
use std::process::ExitCode;

fn main() -> ExitCode {
    match parse_args(std::env::args().skip(1)) {
        Ok(Parsed::Help) => {
            print!("{USAGE}");
            ExitCode::SUCCESS
        }
        Ok(Parsed::Version) => {
            println!("cubicle {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Ok(Parsed::Run(cfg)) => match cubicle::app::run(&cfg) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("cubicle: {e}");
                ExitCode::from(1)
            }
        },
        Err(e) => {
            eprintln!("cubicle: {e}");
            ExitCode::from(2)
        }
    }
}
