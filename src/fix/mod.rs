pub mod dry_run;
pub mod insertion;
pub mod path;

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use oxc_allocator::Allocator;

use crate::config::Config;
use crate::engine::parser::{extract_mocks, parse_source};
use crate::rule::Diagnostic;

use insertion::MockInsertion;

/// Apply fixes to test files based on diagnostics.
pub fn apply_fixes(
    diagnostics: &[Diagnostic],
    config: &Config,
) -> Result<Vec<FixResult>, String> {
    // Group diagnostics by test file
    let mut by_test_file: std::collections::HashMap<PathBuf, Vec<&Diagnostic>> =
        std::collections::HashMap::new();

    for d in diagnostics {
        // The diagnostic's file_path is the source file with hazard.
        // We need to find which test files import it — for now, use the
        // diagnostic's file_path if it's a test file (from mock-consensus),
        // otherwise skip (auto-fix only works for direct test file diagnostics in v1).
        if crate::engine::context::is_test_file_path(&d.file_path) {
            by_test_file
                .entry(d.file_path.clone())
                .or_default()
                .push(d);
        }
    }

    let mut results = Vec::new();

    for (test_file, diags) in &by_test_file {
        match apply_fix_to_file(test_file, diags, config) {
            Ok(result) => results.push(result),
            Err(e) => {
                eprintln!("Warning: Failed to fix {}: {}", test_file.display(), e);
            }
        }
    }

    Ok(results)
}

/// Apply fixes to a single test file.
fn apply_fix_to_file(
    test_file: &Path,
    diagnostics: &[&Diagnostic],
    config: &Config,
) -> Result<FixResult, String> {
    let source = std::fs::read_to_string(test_file)
        .map_err(|e| format!("Failed to read {}: {}", test_file.display(), e))?;

    let allocator = Allocator::default();
    let parse_result = parse_source(&allocator, &source, test_file);

    if parse_result.panicked {
        return Err(format!(
            "Failed to parse {}: parse error",
            test_file.display()
        ));
    }

    // Collect existing mocks (for idempotency)
    let existing_mocks = extract_mocks(&parse_result.program, config.framework);
    let existing_mock_sources: HashSet<String> =
        existing_mocks.iter().map(|m| m.source.clone()).collect();

    // Generate mock insertions for diagnostics that need them (deduplicated)
    // fix.text contains absolute paths — convert to relative format for dedup
    // comparison against existing_mock_sources which are relative (e.g., '../../config')
    let mut seen_modules: HashSet<String> = HashSet::new();
    let insertions: Vec<MockInsertion> = diagnostics
        .iter()
        .filter_map(|d| {
            if let Some(ref fix) = d.fix {
                let rel_source =
                    path::compute_relative_path(test_file, Path::new(&fix.text));
                Some(MockInsertion {
                    module_source: rel_source,
                    span: fix.span,
                })
            } else {
                None
            }
        })
        .filter(|ins| !existing_mock_sources.contains(&ins.module_source))
        .filter(|ins| seen_modules.insert(ins.module_source.clone()))
        .collect();

    if insertions.is_empty() {
        return Ok(FixResult {
            file_path: test_file.to_path_buf(),
            original: source.clone(),
            fixed: source,
            insertions_count: 0,
        });
    }

    // Find insertion point
    let insert_offset =
        insertion::find_insertion_point(&parse_result.program, config.framework);

    // Generate mock statements (module_source is already a relative path)
    let mock_fn = config.framework.mock_fn_name();
    let mut mock_text = String::new();
    for ins in &insertions {
        mock_text.push_str(&format!("\n{}('{}');", mock_fn, ins.module_source));
    }

    // Apply insertion
    let text_insertion = crate::engine::fixer::TextInsertion {
        offset: insert_offset,
        text: mock_text,
    };

    let fixed = crate::engine::fixer::apply_insertions(&source, vec![text_insertion]);

    Ok(FixResult {
        file_path: test_file.to_path_buf(),
        original: source,
        fixed,
        insertions_count: insertions.len(),
    })
}

/// Result of applying fixes to a file.
pub struct FixResult {
    pub file_path: PathBuf,
    pub original: String,
    pub fixed: String,
    pub insertions_count: usize,
}

impl FixResult {
    pub fn has_changes(&self) -> bool {
        self.original != self.fixed
    }
}
