use clap::{Parser, Subcommand, ValueEnum};
use snout_core::{
    analyze_path, compare::compare, format_bytes as bytes, stack::analyze_stack, AnalysisOptions,
};
use std::{
    io::{self, Write},
    path::PathBuf,
    process::ExitCode,
};

#[derive(Parser)]
#[command(
    name = "snout-cli",
    version,
    about = "Rusty's Snout — understand firmware memory usage"
)]
struct Args {
    #[command(subcommand)]
    command: Command,
    #[arg(long, global = true, value_enum, default_value = "text")]
    format: Format,
}
#[derive(Clone, Copy, ValueEnum)]
enum Format {
    Text,
    Json,
}
#[derive(Subcommand)]
enum Command {
    /// Overall Flash, static RAM, metadata and sections.
    Analyze {
        elf: PathBuf,
        /// Linker map supplying capacities and optional --cref dependencies.
        #[arg(long)]
        map: Option<PathBuf>,
    },
    /// Memory attributed to files; includes unattributed bytes.
    Files {
        elf: PathBuf,
        /// Linker map supplying capacities and optional --cref dependencies.
        #[arg(long)]
        map: Option<PathBuf>,
    },
    /// Defined symbols with unique memory contributions.
    Symbols {
        elf: PathBuf,
        /// Linker map supplying capacities and optional --cref dependencies.
        #[arg(long)]
        map: Option<PathBuf>,
    },
    /// Compare memory usage between linked builds.
    Diff {
        old: PathBuf,
        new: PathBuf,
        /// Linker map for the old ELF.
        #[arg(long)]
        old_map: Option<PathBuf>,
        /// Linker map for the new ELF.
        #[arg(long)]
        new_map: Option<PathBuf>,
    },
    /// Read compiler-reported local stack frames (.su).
    Stack {
        elf: PathBuf,
        #[arg(long)]
        stack_usage: PathBuf,
        /// Linker map supplying capacities and optional --cref dependencies.
        #[arg(long)]
        map: Option<PathBuf>,
    },
}

/// Explicit maps supply capacities where supported and cross references where present.
fn analyze_input(
    path: &std::path::Path,
    map: Option<&std::path::Path>,
) -> Result<snout_core::Analysis, Box<dyn std::error::Error>> {
    use snout_core::map::{detect_map_format, parse_map_regions, MapFormat};
    let Some(map) = map else {
        return Ok(analyze_path(path, &AnalysisOptions::default())?);
    };
    let text = std::fs::read_to_string(map).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!(
                "Cannot read linker map {}: {error}; check --map path and permissions",
                map.display()
            ),
        )
    })?;
    let format = detect_map_format(&text);
    let options = match format {
        MapFormat::LlvmLld => AnalysisOptions::default(),
        _ => parse_map_regions(&text).map_err(|error| {
            format!(
                "{}: {error}; supply a valid linker map from this build",
                map.display()
            )
        })?,
    };
    let mut analysis = analyze_path(path, &options)?;
    if format == MapFormat::LlvmLld {
        analysis.warnings.push(
            "LLVM lld maps do not contain physical memory capacities; capacity remains unknown."
                .into(),
        );
    } else {
        analysis.warnings.push(format!("Memory capacities imported from {} ({}). Verify this map belongs to the firmware; memory roles are inferred from names and attributes.", map.display(), format.label()));
    }
    if text
        .lines()
        .any(|line| line.trim() == "Cross Reference Table")
    {
        analysis.dependencies =
            snout_core::dependencies::from_map(&analysis, &text, &map.display().to_string())
                .map_err(|error| {
                    format!(
                        "{}: {error}; regenerate the map with --cref,--no-demangle",
                        map.display()
                    )
                })?;
    } else {
        snout_core::dependencies::import_map(&mut analysis, &text, &map.display().to_string());
    }
    analysis.dependencies.map_path = Some(map.display().to_string());
    Ok(analysis)
}

fn run(args: Args) -> Result<(), Box<dyn std::error::Error>> {
    let mut out = io::BufWriter::new(io::stdout().lock());
    if let Command::Diff {
        old,
        new,
        old_map,
        new_map,
    } = args.command
    {
        let old = analyze_input(&old, old_map.as_deref())?;
        let new = analyze_input(&new, new_map.as_deref())?;
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
        let (path, map) = match &args.command {
            Command::Analyze { elf, map }
            | Command::Files { elf, map }
            | Command::Symbols { elf, map }
            | Command::Stack { elf, map, .. } => (elf, map.as_deref()),
            _ => unreachable!(),
        };
        let analysis = analyze_input(path, map)?;
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
            if let Some(tls) = &analysis.tls {
                writeln!(out, "TLS template per thread: {} ({} initialized, {} zero-initialized), alignment {} B", bytes(tls.template_size), bytes(tls.initialized_size), bytes(tls.zero_initialized_size), tls.alignment)?;
                writeln!(
                    out,
                    "Total TLS RAM: unknown; static RAM excludes TLS templates."
                )?;
                if matches!(
                    args.command,
                    Command::Analyze { .. } | Command::Symbols { .. }
                ) {
                    for symbol in &tls.symbols {
                        writeln!(
                            out,
                            "  TLS +{:#x}: {}  {} [{}]",
                            symbol.offset,
                            bytes(symbol.size),
                            symbol.name,
                            symbol.section
                        )?;
                    }
                }
                writeln!(out)?;
            }
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
