use serde::Serialize;

use crate::engine::EngineResult;
use crate::reporter::{offset_to_location, Reporter};
use crate::rule::Severity;

pub struct JsonReporter {
    pub project_root: std::path::PathBuf,
}

#[derive(Serialize)]
struct JsonReport {
    version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    tsconfig: Option<String>,
    files_checked: usize,
    files_passed: usize,
    files_failed: usize,
    error_count: usize,
    warning_count: usize,
    fixable_count: usize,
    diagnostics: Vec<JsonDiagnostic>,
}

#[derive(Serialize)]
struct JsonDiagnostic {
    rule: String,
    severity: String,
    message: String,
    file: String,
    line: Option<usize>,
    column: Option<usize>,
    help: Option<String>,
    fixable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    suggestion: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    import_chain: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    hazard_sources: Vec<JsonHazardSource>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    hazardous_imports: Vec<JsonHazardousImport>,
}

#[derive(Serialize)]
struct JsonHazardousImport {
    symbol_name: String,
    impact: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    referenced_bindings: Vec<String>,
}

#[derive(Serialize)]
struct JsonHazardSource {
    file: String,
    line: Option<usize>,
    column: Option<usize>,
    message: String,
    category: String,
}

impl Reporter for JsonReporter {
    fn report(&self, result: &EngineResult) {
        let diagnostics: Vec<JsonDiagnostic> = result
            .diagnostics
            .iter()
            .filter(|d| d.severity != Severity::Off)
            .map(|d| {
                let rel_path = pathdiff::diff_paths(&d.file_path, &self.project_root)
                    .unwrap_or_else(|| d.file_path.clone());

                let (line, column) = if d.span.start > 0 {
                    let source = std::fs::read_to_string(&d.file_path).unwrap_or_default();
                    let (l, c, _, _) = offset_to_location(&source, d.span.start);
                    (Some(l), Some(c))
                } else {
                    (None, None)
                };

                let import_chain = d.import_chain.as_ref().map(|chain| {
                    chain
                        .iter()
                        .map(|p| {
                            p.file_name()
                                .and_then(|n| n.to_str())
                                .unwrap_or("?")
                                .to_string()
                        })
                        .collect()
                });

                let hazard_sources: Vec<JsonHazardSource> = d.hazard_sources.iter().map(|hs| {
                    let hs_rel = pathdiff::diff_paths(&hs.file_path, &self.project_root)
                        .unwrap_or_else(|| hs.file_path.clone());
                    let (hs_line, hs_col) = if hs.span.start > 0 {
                        let src = std::fs::read_to_string(&hs.file_path).unwrap_or_default();
                        let (l, c, _, _) = offset_to_location(&src, hs.span.start);
                        (Some(l), Some(c))
                    } else {
                        (None, None)
                    };
                    JsonHazardSource {
                        file: hs_rel.to_string_lossy().to_string(),
                        line: hs_line,
                        column: hs_col,
                        message: hs.message.clone(),
                        category: hs.category.to_string(),
                    }
                }).collect();

                let suggestion = if d.fix.is_none() && d.rule_name == "hazard-reachability" {
                    d.help.clone()
                } else {
                    None
                };

                let hazardous_imports: Vec<JsonHazardousImport> = d
                    .hazardous_imports
                    .iter()
                    .map(|hi| JsonHazardousImport {
                        symbol_name: hi.symbol_name.clone(),
                        impact: hi.impact.to_string(),
                        referenced_bindings: hi.referenced_bindings.clone(),
                    })
                    .collect();

                JsonDiagnostic {
                    rule: d.rule_name.clone(),
                    severity: d.severity.to_string(),
                    message: d.message.clone(),
                    file: rel_path.to_string_lossy().to_string(),
                    line,
                    column,
                    help: d.help.clone(),
                    fixable: d.fix.is_some(),
                    suggestion,
                    import_chain,
                    hazard_sources,
                    hazardous_imports,
                }
            })
            .collect();

        let tsconfig = result.tsconfig_path.as_ref().map(|p| {
            pathdiff::diff_paths(p, &self.project_root)
                .unwrap_or_else(|| p.clone())
                .to_string_lossy()
                .to_string()
        });

        let error_count = result
            .diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Error)
            .count();
        let warning_count = result
            .diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Warning)
            .count();
        let fixable_count = result
            .diagnostics
            .iter()
            .filter(|d| d.severity != Severity::Off && d.fix.is_some())
            .count();

        let report = JsonReport {
            version: "0.1.0".to_string(),
            tsconfig,
            files_checked: result.files_checked,
            files_passed: result.files_passed,
            files_failed: result.files_failed,
            error_count,
            warning_count,
            fixable_count,
            diagnostics,
        };

        println!("{}", serde_json::to_string_pretty(&report).unwrap());
    }
}
