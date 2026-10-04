use clap::{Parser, Subcommand, ValueEnum};
use firmware_analysis_core::{
    analyze_path, compare::compare, format_bytes as bytes, stack::analyze_stack, AnalysisOptions,
};
use std::{
    io::{self, Write},
    path::PathBuf,
    process::ExitCode,
};

#[derive(Parser)]
#[command(
    name = "firmware-explorer",
    version,
    about = "Rusty's Snout — understand firmware memory usage"
)]
struct Args {
    #[command(subcommand)]
    command: Command,
    #[arg(long, global = true, value_enum, default_value = "text")]
    format: Format,
    /// JSON configuration containing physical memory regions.
    #[arg(long, global = true)]
    config: Option<PathBuf>,
}
#[derive(Clone, Copy, ValueEnum)]
enum Format {
    Text,
    Json,
}
#[derive(Subcommand)]
enum Command {
    /// Overall Flash, static RAM, metadata and sections.
    Analyze { elf: PathBuf },
    /// Memory attributed to files; includes unattributed bytes.
    Files { elf: PathBuf },
    /// Defined symbols with unique memory contributions.
    Symbols { elf: PathBuf },
    /// Compare memory usage between linked builds.
    Diff { old: PathBuf, new: PathBuf },
    /// Read compiler-reported local stack frames (.su).
    Stack {
        elf: PathBuf,
        #[arg(long)]
        stack_usage: PathBuf,
    },
}

fn run(args: Args) -> Result<(), Box<dyn std::error::Error>> {
    let options = match args.config {
        Some(p) => serde_json::from_slice(&std::fs::read(p)?)?,
        None => AnalysisOptions::default(),
    };
    let mut out = io::BufWriter::new(io::stdout().lock());
    if let Command::Diff { old, new } = args.command {
        let old = analyze_path(old, &options)?;
        let new = analyze_path(new, &options)?;
        let diff = compare(&old, &new);
        if matches!(args.format, Format::Json) {
            serde_json::to_writer_pretty(&mut out, &diff)?;
            writeln!(out)?;
        } else {
            writeln!(
                out,
                "{} -> {}\nFlash: {} -> {} ({:+} B)\nRAM:   {} -> {} ({:+} B)",
                diff.old_path,
                diff.new_path,
                bytes(diff.old.flash),
                bytes(diff.new.flash),
                diff.flash_delta,
                bytes(diff.old.ram),
                bytes(diff.new.ram),
                diff.ram_delta
            )?;
            for (title, changes) in [
                ("Changed sections", &diff.sections),
                ("Changed files", &diff.files),
                ("Changed symbols", &diff.symbols),
            ] {
                writeln!(
                    out,
                    "\n{title}\n{:>14} {:>14}  Identity",
                    "Flash delta", "RAM delta"
                )?;
                for c in changes {
                    writeln!(
                        out,
                        "{:+14} {:+14}  {} ({})",
                        c.flash_delta, c.ram_delta, c.identity, c.status
                    )?;
                }
            }
            for warning in &diff.warnings {
                writeln!(out, "Note: {warning}")?;
            }
        }
    } else {
        let path = match &args.command {
            Command::Analyze { elf }
            | Command::Files { elf }
            | Command::Symbols { elf }
            | Command::Stack { elf, .. } => elf,
            _ => unreachable!(),
        };
        let analysis = analyze_path(path, &options)?;
        if let Command::Stack { stack_usage, .. } = &args.command {
            let report = analyze_stack(&analysis, stack_usage)?;
            if matches!(args.format, Format::Json) {
                serde_json::to_writer_pretty(&mut out, &report)?;
                writeln!(out)?;
            } else {
                writeln!(
                    out,
                    "Local stack (compiler reported; call-chain total unknown)"
                )?;
                for e in &report.entries {
                    writeln!(
                        out,
                        "{:>12}  {:16} {} ({}:{}, {} ELF matches)",
                        bytes(e.local_bytes),
                        e.qualifier,
                        e.function,
                        e.source_file,
                        e.source_line,
                        e.symbol_candidates.len()
                    )?;
                }
                for warning in &report.warnings {
                    writeln!(out, "Note: {warning}")?;
                }
            }
        } else if matches!(args.format, Format::Json) {
            // All commands share a versioned report envelope, retaining assumptions and metadata.
            serde_json::to_writer_pretty(&mut out, &analysis)?;
            writeln!(out)?;
        } else {
            match args.command {
                Command::Analyze { .. } => {
                    writeln!(out, "Rusty's Snout — Firmware Overview\n{}\nFlash payload: {}\nRAM at runtime (static): {}\nELF file size: {}\n{} / {}-bit / {} endian\nEntry point: {:#x}\n", analysis.path, bytes(analysis.totals.flash), bytes(analysis.totals.ram), bytes(analysis.metadata.file_size), analysis.metadata.architecture, analysis.metadata.bitness, analysis.metadata.endianness, analysis.metadata.entry_point)?;
                    writeln!(
                        out,
                        "{:24} {:>12} {:>12} {:>12} {:>12}",
                        "Section", "Load", "Runtime", "Flash", "RAM"
                    )?;
                    for s in analysis.sections.iter().filter(|s| s.allocated) {
                        writeln!(
                            out,
                            "{:24} {:>12} {:>12} {:>12} {:>12}",
                            s.name,
                            bytes(s.load_size),
                            bytes(s.runtime_size),
                            bytes(s.usage.flash),
                            bytes(s.usage.ram)
                        )?;
                    }
                }
                Command::Files { .. } => {
                    writeln!(out, "{:>12} {:>12}  File / attribution", "Flash", "RAM")?;
                    for f in &analysis.files {
                        writeln!(
                            out,
                            "{:>12} {:>12}  {} [{}]",
                            bytes(f.usage.flash),
                            bytes(f.usage.ram),
                            f.path,
                            f.attribution
                        )?;
                    }
                }
                Command::Symbols { .. } => {
                    writeln!(
                        out,
                        "{:>12} {:>12} {:>12}  Symbol / section",
                        "ELF size", "Flash", "RAM"
                    )?;
                    for s in &analysis.symbols {
                        writeln!(
                            out,
                            "{:>12} {:>12} {:>12}  {} [{}]",
                            bytes(s.size),
                            bytes(s.usage.flash),
                            bytes(s.usage.ram),
                            s.demangled_name,
                            s.section
                        )?;
                    }
                }
                _ => {}
            }
            for warning in &analysis.warnings {
                writeln!(out, "Note: {warning}")?;
            }
        }
    }
    out.flush()?;
    Ok(())
}

fn main() -> ExitCode {
    match run(Args::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            if e.downcast_ref::<io::Error>()
                .is_some_and(|e| e.kind() == io::ErrorKind::BrokenPipe)
                || e.downcast_ref::<serde_json::Error>()
                    .is_some_and(|e| e.io_error_kind() == Some(io::ErrorKind::BrokenPipe))
            {
                return ExitCode::SUCCESS;
            }
            eprintln!("Error: {e}");
            ExitCode::FAILURE
        }
    }
}
