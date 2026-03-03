use std::collections::HashSet;
use std::path::PathBuf;

use crate::engine::context::{GraphContext, MutationImpact, SafeSignal};
use crate::engine::{
    is_module_under_test,
    get_import_specifiers_for_module,
    filter_analyses_by_imports,
    resolve_reexport_impact,
};
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

            // Collect direct (non-type-only) import targets for this test file
            let direct_imports: HashSet<&PathBuf> = ctx
                .graph
                .outgoing_edges(test_file)
                .iter()
                .filter(|e| !e.is_type_only)
                .map(|e| &e.target)
                .collect();

            // Check each module in the effective subgraph
            for module_path in &effective {
                // Skip the test file itself
                if module_path == test_file {
                    continue;
                }

                // Skip the module under test (e.g., foo.test.ts → foo.ts)
                if is_module_under_test(test_file, module_path) {
                    continue;
                }

                // Skip modules that have no hazards and no unsafe exports
                let has_own_hazards = ctx.module_hazards.contains_key(module_path);
                let has_unsafe_exports = ctx.export_analyses.get(module_path).map_or(false, |analyses| {
                    analyses.iter().any(|a| {
                        let effective = resolve_reexport_impact(
                            a, module_path, &ctx.module_hazards, &ctx.all_imports
                        );
                        !matches!(effective, MutationImpact::Safe)
                    })
                });
                if !has_own_hazards && !has_unsafe_exports {
                    continue;
                }

                // Check if this module is in the global mock registry
                if let Some(mock_entries) = ctx.mock_registry.get(module_path) {
                    // This module has been mocked in at least one test
                    if !mocked_by_this_test.contains(module_path) {
                        // Export-level filtering: skip if test only imports safe exports
                        if direct_imports.contains(module_path) {
                            if let Some(analyses) = ctx.export_analyses.get(module_path) {
                                let import_names = get_import_specifiers_for_module(
                                    &ctx.all_imports, test_file, module_path
                                );
                                if !import_names.is_empty() {
                                    let matched = filter_analyses_by_imports(&import_names, analyses);
                                    let all_safe = matched.iter().all(|a| {
                                        let effective = resolve_reexport_impact(
                                            a, module_path, &ctx.module_hazards, &ctx.all_imports
                                        );
                                        effective == MutationImpact::Safe
                                    });
                                    if all_safe {
                                        continue; // Safe imports only → no diagnostic needed
                                    }
                                }
                            }
                        }

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

                        // Only generate fix for modules the test directly imports.
                        // Transitive modules get the diagnostic (informational) but no auto-fix.
                        let fix = if direct_imports.contains(module_path) {
                            Some(Fix {
                                text: module_path.to_string_lossy().to_string(),
                                span: oxc_span::Span::default(),
                            })
                        } else {
                            None
                        };

                        let has_cleanup = test_ctx.safe_signals.iter().any(|s| {
                            matches!(s, SafeSignal::RestoreAllMocks | SafeSignal::ClearAllMocks | SafeSignal::ResetAllMocks)
                        });

                        let help = if has_cleanup {
                            "This module is mocked in other tests. Mock cleanup is present but doesn't cover unmocked modules. Add a mock or add to allowlist.".to_string()
                        } else {
                            "This module is mocked in other tests, suggesting it needs isolation. Add a mock or add to allowlist.".to_string()
                        };

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
                            help: Some(help),
                            fix,
                            import_chain: None,
                            hazard_sources: vec![],
                            hazardous_imports: vec![],
                        });
                    }
                }
            }
        }

        diagnostics
    }
}
