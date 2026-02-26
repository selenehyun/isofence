use owo_colors::OwoColorize;
use similar::{ChangeTag, TextDiff};
use std::path::Path;

use super::FixResult;

/// Print a unified diff for a fix result.
pub fn print_diff(result: &FixResult, project_root: &Path) {
    if !result.has_changes() {
        return;
    }

    let rel_path = pathdiff::diff_paths(&result.file_path, project_root)
        .unwrap_or_else(|| result.file_path.clone());

    println!("{}", format!("--- {}", rel_path.display()).red());
    println!("{}", format!("+++ {}", rel_path.display()).green());

    let diff = TextDiff::from_lines(&result.original, &result.fixed);

    for (idx, group) in diff.grouped_ops(3).iter().enumerate() {
        if idx > 0 {
            println!("{}", "---".dimmed());
        }

        for op in group {
            for change in diff.iter_inline_changes(op) {
                let (sign, _style) = match change.tag() {
                    ChangeTag::Delete => ("-", "red"),
                    ChangeTag::Insert => ("+", "green"),
                    ChangeTag::Equal => (" ", ""),
                };

                let line = change
                    .iter_strings_lossy()
                    .map(|(_, s)| s.to_string())
                    .collect::<String>();

                match change.tag() {
                    ChangeTag::Delete => print!("{}{}", sign.red(), line.red()),
                    ChangeTag::Insert => print!("{}{}", sign.green(), line.green()),
                    ChangeTag::Equal => print!("{}{}", sign, line),
                }
            }
        }
    }

    println!();
}
