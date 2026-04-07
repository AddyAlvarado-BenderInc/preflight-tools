use remove_outside_objects_pdf::process_pdf;
use std::path::{Path, PathBuf};
use std::process;

fn parse_batch_args(args: &[String]) -> Option<(Vec<PathBuf>, Option<PathBuf>)> {
    if !args.get(1).map_or(false, |arg| arg.starts_with('[')) {
        return None;
    }

    let mut raw_tokens: Vec<String> = Vec::new();
    let mut closing_idx: Option<usize> = None;

    for (i, arg) in args[1..].iter().enumerate() {
        raw_tokens.push(arg.clone());
        if arg.ends_with(']') {
            closing_idx = Some(i + 1);
            break;
        }
    }

    let closing_idx = closing_idx?;

    let joined = raw_tokens.join(" ");
    let inner = joined.trim_start_matches('[').trim_end_matches(']');

    let inputs: Vec<PathBuf> = inner
        .split(',')
        .map(|s| PathBuf::from(s.trim()))
        .filter(|p| !p.as_os_str().is_empty())
        .collect();
    if inputs.is_empty() {
        return None;
    }

    let output_dir = args.get(closing_idx + 1).map(PathBuf::from);

    Some((inputs, output_dir))
}

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.contains(&String::from("--version")) {
        let version = env!("CARGO_PKG_VERSION");
        println!("{}", version);
        process::exit(0);
    }

    if let Some((inputs, output_dir)) = parse_batch_args(&args) {
        let mut had_error = false;

        for input_path in inputs {
            if !input_path.exists() {
                eprintln!("Error: {:?} does not exist", input_path.display());
                had_error = true;
                continue;
            }

            let stem = input_path.file_stem().unwrap_or_default().to_string_lossy();
            let output_path = match &output_dir {
                Some(dir) => dir.join(format!("{}-trimmed.pdf", stem)),
                None => {
                    let parent = input_path.parent().unwrap_or(Path::new("."));
                    parent.join(format!("{}-trimmed.pdf", stem))
                }
            };

            if let Some(parent) = output_path.parent() {
                if !parent.exists() {
                    if let Err(e) = std::fs::create_dir_all(parent) {
                        eprintln!("Error: could not create output directory {}", e);
                        had_error = true;
                        continue;
                    }
                }
            }

            match process_pdf(&input_path, &output_path) {
                Ok(()) => println!("Saved: {}", output_path.display()),
                Err(e) => {
                    eprintln!("Error processing {}: {}", input_path.display(), e);
                    had_error = true;
                }
            }
        }

        if had_error {
            process::exit(1);
        }
    } else {
        if args.len() < 2 || args.len() > 3 {
            eprintln!("Usage: {} <input.pdf> [output.pdf]", args[0]);
            eprintln!("       {} [a.pdf, b.pdf, ...] [output/dir]", args[0]);
            process::exit(1);
        }
        let input_path = Path::new(&args[1]);

        if !input_path.exists() {
            eprintln!("Error: {:?} does not exist", input_path.display());
            process::exit(1);
        }

        let output_path: PathBuf = if args.len() == 3 {
            let given = PathBuf::from(&args[2]);
            if given.is_dir() {
                let stem = input_path.file_stem().unwrap_or_default().to_string_lossy();
                given.join(format!("{}-trimmed.pdf", stem))
            } else {
                given
            }
        } else {
            let stem = input_path.file_stem().unwrap_or_default().to_string_lossy();
            let input_dir = input_path.parent().unwrap_or(Path::new("."));
            input_dir.join(format!("{}-trimmed.pdf", stem))
        };

        if let Some(parent) = output_path.parent() {
            if !parent.exists() {
                if let Err(e) = std::fs::create_dir_all(parent) {
                    eprintln!("Error: could not create output directory {}", e);
                    process::exit(1);
                }
            }
        }
        match process_pdf(&input_path, &output_path) {
            Ok(()) => println!("Saved: {}", output_path.display()),
            Err(e) => {
                eprintln!("Error processing {}: {}", input_path.display(), e);
                process::exit(1);
            }
        }
    }
}
