use remove_outside_objects_pdf::process_pdf;
use std::path::{Path, PathBuf};
use std::process;

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 || args.len() > 3 {
        eprintln!("Usage: {} <input.pdf> [output.pdf]", args[0]);
        eprintln!();
        eprintln!("  <input.pdf>   Path to the PDF file to process");
        eprintln!(
            "  [output.pdf]  Path for the output file (default: <input>-trimmed.pdf beside input)"
        );
        process::exit(1);
    }

    let input_path = Path::new(&args[1]);

    if !input_path.exists() {
        eprintln!("Error: input file not found: {}", input_path.display());
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
                eprintln!("Error: could not create output directory: {}", e);
                process::exit(1);
            }
        }
    }

    match process_pdf(input_path, &output_path) {
        Ok(()) => println!("Saved trimmed PDF to {}", output_path.display()),
        Err(e) => {
            eprintln!("Error processing PDF: {}", e);
            process::exit(1);
        }
    }
}
