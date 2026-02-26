use owo_colors::OwoColorize;
use std::collections::HashMap;
use std::path::Path;

use crate::engine::EngineResult;
use crate::reporter::Reporter;
use crate::rule::{Diagnostic, Severity};

pub struct ConsoleReporter {
    pub project_root: std::path::PathBuf,
    pub quiet: bool,
}

impl Reporter for ConsoleReporter {
    fn report(&self, result: &EngineResult) {
        println!("{}", "isofence v0.1.0".bold());
        println!();

        // Group diagnostics by file
        let mut by_file: HashMap<&Path, Vec<&Diagnostic>> = HashMap::new();
        for d in &result.diagnostics {
            by_file.entry(&d.file_path).or_default().push(d);
        }

        if by_file.is_empty() && !self.quiet {
            println!(
                "{} All {} files passed — no isolation issues found.",
                "✓".green().bold(),
                result.files_checked
            );
            return;
        }

        // Sort files for deterministic output
        let mut files: Vec<_> = by_file.keys().collect();
        files.sort();

        for file in files {
            let diagnostics = &by_file[file];
            let rel_path = pathdiff::diff_paths(file, &self.project_root)
                .unwrap_or_else(|| file.to_path_buf());

            let has_errors = diagnostics
                .iter()
                .any(|d| d.severity == Severity::Error);

            if has_errors {
                println!(
                    "{} {}",
                    "✗".red().bold(),
                    rel_path.display().to_string().bold()
                );
            } else {
                println!(
                    "{} {}",
                    "⚠".yellow().bold(),
                    rel_path.display().to_string().bold()
                );
            }
            println!();

            // Group by rule name
            let mut by_rule: HashMap<&str, Vec<&Diagnostic>> = HashMap::new();
            for d in diagnostics {
                by_rule.entry(&d.rule_name).or_default().push(d);
            }

            let mut rules: Vec<_> = by_rule.keys().collect();
            rules.sort();

            for rule_name in rules {
                let diags = &by_rule[rule_name];
                for (i, d) in diags.iter().enumerate() {
                    let prefix = if i == diags.len() - 1 { "└──" } else { "├──" };
                    let severity_str = match d.severity {
                        Severity::Error => "⚠".red().to_string(),
                        Severity::Warning => "⚠".yellow().to_string(),
                        Severity::Off => continue,
                    };

                    // Format source location
                    let location = if d.span.start > 0 {
                        let line = count_lines(&std::fs::read_to_string(file).unwrap_or_default(), d.span.start);
                        format!(" [line {}]", line)
                    } else {
                        String::new()
                    };

                    println!(
                        "  {prefix} {severity_str} {}{}: {}",
                        d.rule_name.dimmed(),
                        location.dimmed(),
                        d.message
                    );

                    if let Some(ref help) = d.help {
                        let help_prefix = if i == diags.len() - 1 { "    " } else { "│   " };
                        println!("  {help_prefix} {}", format!("→ {help}").dimmed());
                    }
                }
            }
            println!();
        }

        // Summary
        println!(
            "{}: {} files checked, {} passed, {} failed",
            "Summary".bold(),
            result.files_checked,
            result.files_passed.to_string().green(),
            result.files_failed.to_string().red()
        );

        // Count by rule
        let mut rule_counts: HashMap<&str, usize> = HashMap::new();
        for d in &result.diagnostics {
            if d.severity != Severity::Off {
                *rule_counts.entry(&d.rule_name).or_default() += 1;
            }
        }

        if !rule_counts.is_empty() {
            let mut counts: Vec<_> = rule_counts.into_iter().collect();
            counts.sort_by_key(|(_, c)| std::cmp::Reverse(*c));

            let parts: Vec<String> = counts
                .iter()
                .map(|(name, count)| format!("{count} {name}"))
                .collect();
            println!("  {}", parts.join(", "));
        }
    }
}

fn count_lines(source: &str, offset: u32) -> usize {
    source[..offset as usize]
        .chars()
        .filter(|&c| c == '\n')
        .count()
        + 1
}
