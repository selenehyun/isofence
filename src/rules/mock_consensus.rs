use std::collections::HashSet;
use std::path::PathBuf;

use crate::engine::context::GraphContext;
use crate::rule::{Diagnostic, Fix, HazardCategory, Rule, RuleMeta, Severity};

/// Mock consensus rule: detects modules that are mocked in some tests but not others.
pub struct MockConsensus;

impl Rule for MockConsensus {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            name: "mock-consensus",
            description: "Module mocked elsewhere but used without mock (mock omission)",
            category: HazardCategory::MutableState,
            default_severity: Severity::Warning,
        }
    }

    fn check_graph(
        &self,
        ctx: &GraphContext,
    ) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();

        // For each test file, compute effective subgraph and check for mock omissions
        for (test_file, test_ctx) in &ctx.test_contexts {
            let effective = ctx.graph.effective_subgraph(test_file, &test_ctx.mocks);

            // Get the set of modules this test file mocks
            let mocked_by_this_test: HashSet<PathBuf> = test_ctx
                .mocks
                .iter()
                .filter_map(|m| m.resolved_path.clone())
                .collect();

            // Check each module in the effective subgraph
            for module_path in &effective {
                // Skip the test file itself
                if module_path == test_file {
                    continue;
                }

                // Check if this module is in the global mock registry
                if let Some(mock_entries) = ctx.mock_registry.get(module_path) {
                    // This module has been mocked in at least one test
                    if !mocked_by_this_test.contains(module_path) {
                        // Mock omission: this test doesn't mock it but others do
                        let mocked_in: Vec<String> = mock_entries
                            .iter()
                            .map(|(path, _)| {
                                path.file_name()
                                    .and_then(|n| n.to_str())
                                    .unwrap_or("unknown")
                                    .to_string()
                            })
                            .collect();

                        let module_name = module_path
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("unknown");

                        diagnostics.push(Diagnostic {
                            rule_name: "mock-consensus".to_string(),
                            severity: Severity::Warning,
                            message: format!(
                                "`{}` is mocked in [{}] but not in this test",
                                module_name,
                                mocked_in.join(", ")
                            ),
                            file_path: test_file.clone(),
                            span: oxc_span::Span::default(),
                            help: Some(
                                "This module is mocked in other tests, suggesting it needs isolation. Add a mock or add to allowlist."
                                    .to_string(),
                            ),
                            fix: Some(Fix {
                                text: module_path.to_string_lossy().to_string(),
                                span: oxc_span::Span::default(),
                            }),
                        });
                    }
                }
            }
        }

        diagnostics
    }
}
