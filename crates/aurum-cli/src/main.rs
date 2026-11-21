//! Entry point of the `aurum` binary: runs the CLI, prints the result to
//! stdout or the error message to stderr (exit code 1).

use std::io::Write;
use std::process::ExitCode;

use aurum_cli::env::Env;

fn main() -> ExitCode {
    let argv: Vec<String> =
        std::env::args_os().skip(1).map(|argument| argument.to_string_lossy().into_owned()).collect();
    let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
    };
    match runtime.block_on(aurum_cli::run(&argv, &Env::from_process())) {
        Ok(output) => {
            // A closed pipe (`aurum ... | head`) is not an error worth reporting.
            let _ = std::io::stdout().write_all(output.as_bytes());
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
