// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! `beamcraft-fixtures`: generate, verify and check the original F07 test fixtures.
//!
//! Exit codes: 0 success or accepted input, 1 verification mismatch or I/O failure,
//! 2 usage error, 3 input rejected.

use std::path::Path;
use std::process::ExitCode;

use beamcraft_fixtures::{ReadError, fixture, generate, read_bounded, verify, write};

const USAGE: &str = "\
usage:
  beamcraft-fixtures generate --out <DIR>   write the fixture set and manifest into DIR
  beamcraft-fixtures verify --dir <DIR>     compare DIR with the generator, byte for byte
  beamcraft-fixtures check <FILE>           validate one beamcraft.fixture/1 description
  beamcraft-fixtures list                   list fixture IDs, support status and paths
  beamcraft-fixtures g11 generate --out <DIR>  write the G11 accuracy set into DIR
  beamcraft-fixtures g11 verify --dir <DIR>    compare DIR with the G11 generator, byte for byte
  beamcraft-fixtures --version";

const FAILURE: u8 = 1;
const USAGE_ERROR: u8 = 2;
const REJECTED: u8 = 3;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).take(8).collect();
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    match words.as_slice() {
        ["generate", "--out", dir] => run_generate(Path::new(dir)),
        ["verify", "--dir", dir] => run_verify(Path::new(dir)),
        ["check", file] => run_check(Path::new(file)),
        ["list"] => run_list(),
        ["g11", "generate", "--out", dir] => run_g11_generate(Path::new(dir)),
        ["g11", "verify", "--dir", dir] => run_g11_verify(Path::new(dir)),
        ["--version" | "-V"] => {
            println!("beamcraft-fixtures {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        ["--help" | "-h"] => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!("{USAGE}");
            ExitCode::from(USAGE_ERROR)
        }
    }
}

fn run_g11_generate(dir: &Path) -> ExitCode {
    match beamcraft_fixtures::g11::write(dir) {
        Ok(count) => {
            println!("generated {count} G11 files in {}", dir.display());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::from(FAILURE)
        }
    }
}

fn run_g11_verify(dir: &Path) -> ExitCode {
    match beamcraft_fixtures::g11::verify(dir) {
        Ok(problems) if problems.is_empty() => {
            println!(
                "verified {} G11 files in {}",
                beamcraft_fixtures::g11::FILES.len(),
                dir.display()
            );
            ExitCode::SUCCESS
        }
        Ok(problems) => {
            for problem in &problems {
                println!("mismatch: {problem}");
            }
            println!("verification failed: {} problems", problems.len());
            ExitCode::from(FAILURE)
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::from(FAILURE)
        }
    }
}

fn run_generate(dir: &Path) -> ExitCode {
    let result = generate().and_then(|set| write(&set, dir).map(|()| set.files().len()));
    match result {
        Ok(count) => {
            println!("generated {count} files in {}", dir.display());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::from(FAILURE)
        }
    }
}

fn run_verify(dir: &Path) -> ExitCode {
    match verify(dir) {
        Ok(report) if report.problems.is_empty() => {
            println!(
                "verified {} files and {} manifest digests in {}",
                report.files_compared,
                report.digests_checked,
                dir.display()
            );
            ExitCode::SUCCESS
        }
        Ok(report) => {
            for problem in &report.problems {
                println!("mismatch: {problem}");
            }
            println!("verification failed: {} problems", report.problems.len());
            ExitCode::from(FAILURE)
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::from(FAILURE)
        }
    }
}

fn run_check(file: &Path) -> ExitCode {
    let bytes = match read_bounded(file, fixture::MAX_INPUT_BYTES) {
        Ok(bytes) => bytes,
        Err(ReadError::TooLarge { limit }) => {
            println!(
                "rejected {}: input is larger than {limit} bytes",
                fixture::ErrorCode::LimitExceeded.as_str()
            );
            return ExitCode::from(REJECTED);
        }
        Err(ReadError::Io(e)) => {
            eprintln!("error: {}: {e}", file.display());
            return ExitCode::from(FAILURE);
        }
    };
    match fixture::check(&bytes) {
        Ok(accepted) => {
            println!(
                "accepted {} plates={} members={}",
                accepted.id, accepted.plates, accepted.members
            );
            ExitCode::SUCCESS
        }
        Err(rejection) => {
            println!("rejected {rejection}");
            ExitCode::from(REJECTED)
        }
    }
}

fn run_list() -> ExitCode {
    match generate() {
        Ok(set) => {
            for case in &set.cases {
                println!("{}\t{}\t{}", case.id, case.support.as_str(), case.path);
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::from(FAILURE)
        }
    }
}
