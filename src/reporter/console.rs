use owo_colors::OwoColorize;
use owo_colors::Stream::Stdout;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::engine::EngineResult;
use crate::reporter::{get_source_line, offset_to_location, Reporter};
use crate::rule::{Diagnostic, HazardSource, Severity};

pub struct ConsoleReporter {
    pub project_root: std::path::PathBuf,
    pub quiet: bool,
    pub show_all: bool,
    pub fix_applied: bool,
}

/// Max number of files shown before truncation (unless --all).
const FILE_LIMIT: usize = 15;

impl Reporter for ConsoleReporter {
    fn report(&self, result: &EngineResult) {
        println!(
            "{}",
            "isofence v0.1.0"
                .if_supports_color(Stdout, |s| s.bold())
        );
        if let Some(ref tsconfig) = result.tsconfig_path {
            let rel = pathdiff::diff_paths(tsconfig, &self.project_root)
                .unwrap_or_else(|| tsconfig.clone());
            println!(
                "  tsconfig: {}",
                rel.display()
                    .to_string()
                    .if_supports_color(Stdout, |s| s.dimmed()),
            );
        }
        println!();

        // Group diagnostics by file
        let mut by_file: HashMap<&Path, Vec<&Diagnostic>> = HashMap::new();
        for d in &result.diagnostics {
            if d.severity == Severity::Off {
                continue;
            }
            by_file.entry(&d.file_path).or_default().push(d);
        }

        if by_file.is_empty() && !self.quiet {
            println!(
                "{} All {} files passed — no isolation issues found.",
                "✓".if_supports_color(Stdout, |s| s.green().bold().to_string())
                    .to_string(),
                result.files_checked
            );
            return;
        }

        // Sort files for deterministic output
        let mut files: Vec<_> = by_file.keys().copied().collect();
        files.sort();

        let total_files_with_issues = files.len();
        let display_count = if self.show_all {
            total_files_with_issues
        } else {
            total_files_with_issues.min(FILE_LIMIT)
        };

        for file in files.iter().take(display_count) {
            let diagnostics = &by_file[file];
            let rel_path = pathdiff::diff_paths(file, &self.project_root)
                .unwrap_or_else(|| file.to_path_buf());

            // Read source file once for all diagnostics in this file
            let source = std::fs::read_to_string(file).unwrap_or_default();

            // Print file header
            println!(
                "{}",
                rel_path
                    .display()
                    .to_string()
                    .if_supports_color(Stdout, |s| s.bold())
            );

            // Sort diagnostics by span start
            let mut sorted_diags: Vec<&Diagnostic> = diagnostics.to_vec();
            sorted_diags.sort_by_key(|d| d.span.start);

            // Compute max line number width for alignment
            let max_line = sorted_diags
                .iter()
                .filter(|d| d.span.start > 0)
                .map(|d| {
                    let (line, _, _, _) = offset_to_location(&source, d.span.start);
                    line
                })
                .max()
                .unwrap_or(1);
            let line_width = digit_count(max_line);

            for d in &sorted_diags {
                if d.span.start > 0 && !source.is_empty() {
                    render_source_context(&source, d, line_width);
                } else {
                    render_fallback(d, line_width, &self.project_root);
                }
            }

            println!();
        }

        // Truncation message
        if !self.show_all && total_files_with_issues > FILE_LIMIT {
            let remaining = total_files_with_issues - FILE_LIMIT;
            println!(
                "... {} more files with issues (--all to expand)",
                remaining
            );
            println!();
        }

        // Summary
        println!(
            "{}: {} files checked, {} passed, {} failed",
            "Summary".if_supports_color(Stdout, |s| s.bold()),
            result.files_checked,
            result
                .files_passed
                .to_string()
                .if_supports_color(Stdout, |s| s.green()),
            result
                .files_failed
                .to_string()
                .if_supports_color(Stdout, |s| s.red()),
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

        // Fix suggestion
        let fixable_count = result
            .diagnostics
            .iter()
            .filter(|d| d.severity != Severity::Off && d.fix.is_some())
            .count();

        if fixable_count > 0 && !self.fix_applied {
            println!();
            println!(
                "  {} {} issue(s) can be auto-fixed. Run `isofence --fix` to apply.",
                "🔧", fixable_count,
            );
            println!(
                "     {}",
                "isofence inserts mock declarations only — run your formatter afterward."
                    .if_supports_color(Stdout, |s| s.dimmed()),
            );
        }
    }
}

/// Render a diagnostic with source context (for diagnostics with valid spans).
fn render_source_context(source: &str, d: &Diagnostic, line_width: usize) {
    let (line, col, line_start, line_end) = offset_to_location(source, d.span.start);
    let raw_line = get_source_line(source, line_start, line_end);

    // Replace tabs with 4 spaces for consistent alignment
    let display_line = raw_line.replace('\t', "    ");

    // Compute column in the display string (accounting for tab expansion)
    let display_col = compute_display_col(raw_line, col);

    // Compute underline length
    let span_len = (d.span.end - d.span.start) as usize;
    // Clamp to end of line for multi-line spans
    let raw_end_on_line = (col - 1 + span_len).min(raw_line.len());
    let underline_display_len =
        compute_display_col(raw_line, raw_end_on_line + 1) - display_col;
    let underline_len = underline_display_len.max(1);

    // Gutter padding
    let gutter = " ".repeat(line_width);

    // Source line
    println!(
        " {line_num} {sep} {code}",
        line_num = format!("{:>width$}", line, width = line_width)
            .if_supports_color(Stdout, |s| s.cyan()),
        sep = "│".if_supports_color(Stdout, |s| s.dimmed()),
        code = display_line,
    );

    // Underline + rule name
    let underline = "~".repeat(underline_len);
    let fix_badge = if d.fix.is_some() { " (fix)" } else { "" };
    let annotation = format!(
        "{spaces}{underline} {rule}{fix_badge}",
        spaces = " ".repeat(display_col - 1),
        underline = underline,
        rule = d.rule_name,
    );
    println!(
        " {gutter} {sep} {annotation}",
        gutter = gutter,
        sep = "│".if_supports_color(Stdout, |s| s.dimmed()),
        annotation = color_by_severity(&annotation, d.severity),
    );

    // Help text
    if let Some(ref help) = d.help {
        println!(
            " {gutter} {} {}",
            "=".if_supports_color(Stdout, |s| s.dimmed()),
            format!("help: {help}").if_supports_color(Stdout, |s| s.dimmed()),
        );
    }
}

/// Render a fallback for diagnostics without source location (e.g., hazard-reachability).
fn render_fallback(d: &Diagnostic, line_width: usize, project_root: &Path) {
    let gutter = " ".repeat(line_width);
    let icon = match d.severity {
        Severity::Error => color_by_severity("⚠", Severity::Error),
        Severity::Warning => color_by_severity("⚠", Severity::Warning),
        Severity::Off => return,
    };
    let fix_badge = if d.fix.is_some() { " (fix)" } else { "" };
    let rule_display = format!("{}{}", d.rule_name, fix_badge);
    println!(
        " {gutter}  {icon} {rule}: {msg}",
        gutter = gutter,
        icon = icon,
        rule = color_by_severity(&rule_display, d.severity),
        msg = d.message,
    );

    // Render import chain if present
    if let Some(ref chain) = d.import_chain {
        let chain_display = format_import_chain(chain, project_root);
        println!(
            " {gutter}    {} {}",
            "via:".if_supports_color(Stdout, |s| s.dimmed()),
            chain_display.if_supports_color(Stdout, |s| s.dimmed()),
        );
    }

    // Render hazard source lines
    for hs in &d.hazard_sources {
        render_hazard_source(&gutter, hs, project_root);
    }

    if let Some(ref help) = d.help {
        println!(
            " {gutter}    {} {}",
            "=".if_supports_color(Stdout, |s| s.dimmed()),
            format!("help: {help}").if_supports_color(Stdout, |s| s.dimmed()),
        );
    }
}

/// Format a file path as a clickable terminal hyperlink (OSC 8).
/// Display shows just the filename, link target is the full file:// URI.
/// Unsupported terminals gracefully show just the display text.
fn terminal_hyperlink(abs_path: &Path, display_name: &str) -> String {
    let uri = format!("file://{}", abs_path.display());
    format!("\x1b]8;;{uri}\x07{display_name}\x1b]8;;\x07")
}

/// Format an import chain as a "→"-separated string with clickable filenames.
fn format_import_chain(chain: &[PathBuf], project_root: &Path) -> String {
    chain
        .iter()
        .map(|p| {
            let abs_path = if p.is_absolute() {
                p.clone()
            } else {
                project_root.join(p)
            };
            let name = p
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("?");
            terminal_hyperlink(&abs_path, name)
        })
        .collect::<Vec<_>>()
        .join(" → ")
}

/// Render a single hazard source line from the hazardous module.
fn render_hazard_source(gutter: &str, hs: &HazardSource, project_root: &Path) {
    if hs.span.start == 0 {
        // No valid span — show message only
        println!(
            " {gutter}    {}",
            hs.message.if_supports_color(Stdout, |s| s.dimmed()),
        );
        return;
    }

    let source = match std::fs::read_to_string(&hs.file_path) {
        Ok(s) => s,
        Err(_) => {
            println!(
                " {gutter}    {}",
                hs.message.if_supports_color(Stdout, |s| s.dimmed()),
            );
            return;
        }
    };

    let (line, col, line_start, line_end) = offset_to_location(&source, hs.span.start);
    let raw_line = get_source_line(&source, line_start, line_end);
    let display_line = raw_line.replace('\t', "    ");
    let display_col = compute_display_col(raw_line, col);

    let span_len = (hs.span.end - hs.span.start) as usize;
    let raw_end_on_line = (col - 1 + span_len).min(raw_line.len());
    let underline_display_len =
        compute_display_col(raw_line, raw_end_on_line + 1) - display_col;
    let underline_len = underline_display_len.max(1);

    let rel = pathdiff::diff_paths(&hs.file_path, project_root)
        .unwrap_or_else(|| hs.file_path.clone());
    let file_label = format!("{}:{}", rel.display(), line);

    // File:line label
    println!(
        " {gutter}    {}",
        file_label.if_supports_color(Stdout, |s| s.dimmed()),
    );

    // Source line
    println!(
        " {gutter}    {line_num} {sep} {code}",
        line_num = format!("{line}").if_supports_color(Stdout, |s| s.cyan()),
        sep = "│".if_supports_color(Stdout, |s| s.dimmed()),
        code = display_line,
    );

    // Underline + hazard message
    let underline = "~".repeat(underline_len);
    let annotation = format!(
        "{spaces}{underline} {msg}",
        spaces = " ".repeat(display_col - 1),
        msg = hs.message,
    );
    println!(
        " {gutter}      {sep} {annotation}",
        sep = "│".if_supports_color(Stdout, |s| s.dimmed()),
        annotation = annotation
            .if_supports_color(Stdout, |s| s.red())
            .to_string(),
    );
}

/// Apply color based on severity.
fn color_by_severity(text: &str, severity: Severity) -> String {
    match severity {
        Severity::Error => text
            .if_supports_color(Stdout, |s| s.red())
            .to_string(),
        Severity::Warning => text
            .if_supports_color(Stdout, |s| s.yellow())
            .to_string(),
        Severity::Off => text.to_string(),
    }
}

/// Compute the display column accounting for tab→4-space expansion.
/// `col` is 1-indexed byte column in the raw line.
fn compute_display_col(raw_line: &str, col: usize) -> usize {
    let mut display = 1;
    for (i, ch) in raw_line.char_indices() {
        if i >= col - 1 {
            break;
        }
        if ch == '\t' {
            display += 4;
        } else {
            display += 1;
        }
    }
    display
}

/// Count the number of digits in a number.
fn digit_count(n: usize) -> usize {
    if n == 0 {
        return 1;
    }
    (n as f64).log10().floor() as usize + 1
}
