use std::path::PathBuf;
use std::process;

use clap::Parser;
use ignore::WalkBuilder;

use isofence::config::{Config, OutputFormat};
use isofence::engine::context::is_test_file_path;
use isofence::engine::Engine;
use isofence::fix;
use isofence::progress::create_progress;
use isofence::reporter::console::ConsoleReporter;
use isofence::reporter::json::JsonReporter;
use isofence::reporter::Reporter;
use isofence::rule::declarative::DeclarativeRule;
use isofence::rule::registry::RuleRegistry;
use isofence::rule::Severity;
use isofence::rules::all_builtin_rules;

/// IsoFence — Reference Graph based test isolation verification for TypeScript
#[derive(Parser)]
#[command(name = "isofence", version, about)]
struct Cli {
    /// Files or directories to check (default: project root)
    #[arg(default_value = ".")]
    paths: Vec<PathBuf>,

    /// Auto-fix: insert missing mock declarations
    #[arg(long)]
    fix: bool,

    /// Show fix preview without applying (use with --fix)
    #[arg(long)]
    dry_run: bool,

    /// Output format
    #[arg(long, default_value = "console")]
    format: String,

    /// Transitive dependency check depth
    #[arg(short, long, default_value = "1")]
    depth: usize,

    /// Treat all findings as errors (for CI)
    #[arg(long)]
    strict: bool,

    /// Disable mock consensus check
    #[arg(long)]
    no_consensus: bool,

    /// Path to tsconfig.json (usually auto-detected)
    #[arg(long)]
    tsconfig: Option<PathBuf>,

    /// Generate isofence.json template
    #[arg(long)]
    init: bool,

    /// Only show files with issues
    #[arg(short, long)]
    quiet: bool,
}

fn main() {
    let cli = Cli::parse();

    if cli.init {
        generate_config_template();
        return;
    }

    // Determine project root
    let project_root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

    // Load config
    let mut config = Config::load(project_root.clone());

    // Apply CLI overrides
    config.depth = cli.depth;
    config.strict = cli.strict;
    config.quiet = cli.quiet;
    config.fix = cli.fix;
    config.dry_run = cli.dry_run;

    if cli.no_consensus {
        config.mock_consensus = false;
    }

    if let Some(tsconfig) = cli.tsconfig {
        config.tsconfig_path = Some(tsconfig);
    }

    config.format = match cli.format.as_str() {
        "json" => OutputFormat::Json,
        _ => OutputFormat::Console,
    };

    // Build rule registry
    let mut registry = RuleRegistry::new();
    registry.register_all(all_builtin_rules());

    // Disable mock-consensus if configured
    if !config.mock_consensus {
        registry.disable("mock-consensus");
    }

    // Apply rule severity overrides from config
    for (name, rule_config) in &config.rule_configs {
        registry.set_severity(name, rule_config.severity);
    }

    // Load custom declarative rules
    for rule_path in &config.custom_rules {
        match DeclarativeRule::load_from_file(rule_path) {
            Ok(rules) => {
                for rule in rules {
                    registry.register(Box::new(rule));
                }
            }
            Err(e) => {
                eprintln!("Warning: Failed to load custom rule {}: {}", rule_path.display(), e);
            }
        }
    }

    // Discover files
    let (test_files, source_files) = discover_files(&cli.paths, &config);

    if test_files.is_empty() {
        if !cli.quiet {
            eprintln!("No test files found. Run `isofence --help` for usage.");
        }
        process::exit(0);
    }

    // Run engine
    let quiet = config.quiet || config.format == OutputFormat::Json;
    let progress = create_progress(quiet);
    let engine = Engine::new(config.clone(), registry);
    let result = engine.run(&test_files, &source_files, &*progress);

    // Apply fixes if requested
    if config.fix {
        match fix::apply_fixes(&result.diagnostics, &config) {
            Ok(fix_results) => {
                for fr in &fix_results {
                    if fr.has_changes() {
                        if config.dry_run {
                            fix::dry_run::print_diff(fr, &project_root);
                        } else {
                            if let Err(e) = std::fs::write(&fr.file_path, &fr.fixed) {
                                eprintln!("Error writing {}: {}", fr.file_path.display(), e);
                            } else {
                                eprintln!(
                                    "Fixed: {} ({} mock(s) inserted)",
                                    fr.file_path.display(),
                                    fr.insertions_count
                                );
                            }
                        }
                    }
                }
            }
            Err(e) => {
                eprintln!("Error applying fixes: {e}");
            }
        }
    }

    // Report
    let reporter: Box<dyn Reporter> = match config.format {
        OutputFormat::Json => Box::new(JsonReporter {
            project_root: project_root.clone(),
        }),
        OutputFormat::Console => Box::new(ConsoleReporter {
            project_root: project_root.clone(),
            quiet: config.quiet,
        }),
    };

    reporter.report(&result);

    // Exit code
    let has_errors = result.diagnostics.iter().any(|d| {
        if config.strict {
            d.severity != Severity::Off
        } else {
            d.severity == Severity::Error
        }
    });

    if has_errors {
        process::exit(1);
    }
}

/// Discover test files and source files from the given paths.
fn discover_files(paths: &[PathBuf], _config: &Config) -> (Vec<PathBuf>, Vec<PathBuf>) {
    let mut test_files = Vec::new();
    let mut source_files = Vec::new();

    for path in paths {
        let walker = WalkBuilder::new(path)
            .hidden(true)
            .git_ignore(true)
            .git_global(true)
            .filter_entry(|entry| {
                let path = entry.path();
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

                // Skip common non-source directories
                if entry.file_type().is_some_and(|ft| ft.is_dir()) {
                    return !matches!(
                        name,
                        "node_modules" | "dist" | "build" | ".git" | "coverage" | ".next"
                    );
                }
                true
            })
            .build();

        for entry in walker.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }

            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if !matches!(ext, "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs") {
                continue;
            }

            // Skip declaration files
            if path.to_string_lossy().ends_with(".d.ts") {
                continue;
            }

            if is_test_file_path(path) {
                test_files.push(path.to_path_buf());
            } else {
                source_files.push(path.to_path_buf());
            }
        }
    }

    test_files.sort();
    source_files.sort();
    (test_files, source_files)
}

fn generate_config_template() {
    let template = r#"{
  // Safe module patterns (always skipped)
  "allowlist": [
    "src/types/**",
    "src/constants/**"
  ],

  // Transitive dependency check depth (default: 1)
  // "depth": 2,

  // Mock consensus check (default: true)
  // "mockConsensus": false,

  // Rule overrides
  // "rules": {
  //   "mutable-module-var": "error",
  //   "top-level-call": "warning",
  //   "iife": "off"
  // },

  // Custom declarative rule files
  // "customRules": ["./isofence-rules/firebase.json"]
}
"#;

    let path = "isofence.json";
    if std::path::Path::new(path).exists() {
        eprintln!("isofence.json already exists. Remove it first to regenerate.");
        process::exit(1);
    }

    std::fs::write(path, template).unwrap_or_else(|e| {
        eprintln!("Error writing isofence.json: {e}");
        process::exit(2);
    });

    println!("Created isofence.json template.");
}
