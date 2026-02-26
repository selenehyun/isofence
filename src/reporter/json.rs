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

                JsonDiagnostic {
                    rule: d.rule_name.clone(),
                    severity: d.severity.to_string(),
                    message: d.message.clone(),
                    file: rel_path.to_string_lossy().to_string(),
                    line,
                    column,
                    help: d.help.clone(),
                }
            })
            .collect();

        let tsconfig = result.tsconfig_path.as_ref().map(|p| {
            pathdiff::diff_paths(p, &self.project_root)
                .unwrap_or_else(|| p.clone())
                .to_string_lossy()
                .to_string()
        });

        let report = JsonReport {
            version: "0.1.0".to_string(),
            tsconfig,
            files_checked: result.files_checked,
            files_passed: result.files_passed,
            files_failed: result.files_failed,
            diagnostics,
        };

        println!("{}", serde_json::to_string_pretty(&report).unwrap());
    }
}
