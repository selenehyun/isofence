pub mod context;
pub mod fixer;
pub mod graph;
pub mod parser;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use oxc_allocator::Allocator;
use oxc_resolver::{ResolveOptions, Resolver, TsconfigOptions, TsconfigReferences};
use rayon::prelude::*;

use crate::config::Config;
use crate::engine::context::{
    EdgeKind, GraphContext, MockKind, ModuleContext, TestContextSummary,
};
use crate::engine::graph::ModuleGraph;
use crate::engine::parser::{
    extract_imports, extract_mocks, extract_safe_signals, parse_source,
};
use crate::progress::{Progress, SilentProgress};
use crate::rule::registry::RuleRegistry;
use crate::rule::{Diagnostic, Fix, Hazard, HazardCategory, Severity};

/// The main analysis engine.
pub struct Engine {
    pub config: Config,
    pub registry: RuleRegistry,
}

/// Result of running the engine.
pub struct EngineResult {
    pub diagnostics: Vec<Diagnostic>,
    pub files_checked: usize,
    pub files_passed: usize,
    pub files_failed: usize,
    pub tsconfig_path: Option<PathBuf>,
}

impl Engine {
    pub fn new(config: Config, registry: RuleRegistry) -> Self {
        Self { config, registry }
    }

    /// Run the full analysis pipeline.
    pub fn run(&self, test_files: &[PathBuf], all_source_files: &[PathBuf], progress: &dyn Progress) -> EngineResult {
        // Phase 1 & 2: Per-file analysis (parallel)
        let total = (test_files.len() + all_source_files.len()) as u64;
        progress.start_file_analysis(total);

        let per_file_results: Vec<FileAnalysisResult> = test_files
            .par_iter()
            .chain(all_source_files.par_iter())
            .filter_map(|file_path| {
                let result = self.analyze_file(file_path);
                progress.file_analyzed();
                result
            })
            .collect();

        // Collect results
        let mut all_diagnostics: Vec<Diagnostic> = Vec::new();
        let mut module_hazards: HashMap<PathBuf, Vec<Hazard>> = HashMap::new();
        let mut test_contexts: HashMap<PathBuf, TestContextSummary> = HashMap::new();
        let mut all_imports: HashMap<PathBuf, Vec<crate::engine::context::ImportInfo>> =
            HashMap::new();

        for result in &per_file_results {
            if result.test_context.is_some() {
                all_diagnostics.extend(result.diagnostics.clone());
            }
            if !result.hazards.is_empty() {
                module_hazards.insert(result.file_path.clone(), result.hazards.clone());
            }
            if let Some(ref ctx) = result.test_context {
                test_contexts.insert(result.file_path.clone(), ctx.clone());
            }
            all_imports.insert(result.file_path.clone(), result.imports.clone());
        }

        // Apply allowlist: remove allowed modules from hazards and diagnostics
        module_hazards.retain(|path, _| !self.config.is_allowed(path));

        // Phase 3: Graph analysis
        progress.start_graph_phase();
        let graph = self.build_graph(&per_file_results, &all_imports);

        // Build mock registry from all test contexts (filtered by allowlist)
        progress.graph_step("Building mock registry...");
        let mut mock_registry = self.build_mock_registry(&test_contexts);
        mock_registry.retain(|path, _| !self.config.is_allowed(path));

        let graph_ctx = GraphContext {
            graph,
            mock_registry,
            module_hazards,
            test_contexts,
        };

        // Run graph-level rules (e.g., mock-consensus)
        progress.graph_step("Running graph rules...");
        let graph_diagnostics = self.run_graph_rules(&graph_ctx);
        all_diagnostics.extend(graph_diagnostics);

        // Hazard reachability: for each test file, find unmocked hazardous modules
        progress.start_reachability(graph_ctx.test_contexts.len() as u64);
        let reachability_diagnostics = self.run_hazard_reachability(&graph_ctx, progress);
        all_diagnostics.extend(reachability_diagnostics);

        // Sort and deduplicate
        all_diagnostics.sort_by(|a, b| {
            a.file_path
                .cmp(&b.file_path)
                .then_with(|| a.span.start.cmp(&b.span.start))
                .then_with(|| a.message.cmp(&b.message))
        });
        all_diagnostics.dedup_by(|a, b| {
            a.file_path == b.file_path
                && a.span == b.span
                && a.rule_name == b.rule_name
                && a.message == b.message
        });

        // Count results
        let files_checked = test_files.len();
        let files_with_issues: std::collections::HashSet<_> = all_diagnostics
            .iter()
            .filter(|d| d.severity == crate::rule::Severity::Error)
            .map(|d| &d.file_path)
            .collect();
        let files_failed = files_with_issues.len();
        let files_passed = files_checked.saturating_sub(files_failed);

        progress.finish();

        EngineResult {
            diagnostics: all_diagnostics,
            files_checked,
            files_passed,
            files_failed,
            tsconfig_path: self.config.tsconfig_path.clone(),
        }
    }

    /// Run the full analysis pipeline with no progress output.
    pub fn run_silent(&self, test_files: &[PathBuf], source_files: &[PathBuf]) -> EngineResult {
        self.run(test_files, source_files, &SilentProgress)
    }

    /// Analyze a single file (Phase 1 & 2).
    fn analyze_file(&self, file_path: &Path) -> Option<FileAnalysisResult> {
        let source_text = std::fs::read_to_string(file_path).ok()?;
        let allocator = Allocator::default();
        let parse_result = parse_source(&allocator, &source_text, file_path);

        if parse_result.panicked || !parse_result.errors.is_empty() {
            // Skip files with parse errors
            return None;
        }

        let program = &parse_result.program;
        let mut ctx = ModuleContext::new(
            file_path.to_path_buf(),
            source_text.clone(),
            self.config.framework,
        );

        // Extract imports and mocks
        ctx.imports = extract_imports(program);
        if ctx.is_test_file {
            ctx.mocks = extract_mocks(program, self.config.framework);
            ctx.safe_signals = extract_safe_signals(program);
        }

        let mut diagnostics = Vec::new();
        let mut hazards = Vec::new();

        // Phase 1: check_module_item for each top-level statement
        for stmt in &program.body {
            for rule in self.registry.enabled_rules() {
                let mut diags = rule.check_module_item(stmt, &ctx);
                // Override severity from config
                for d in &mut diags {
                    if let Some(sev) = self.registry.configured_severity(&rule.meta().name) {
                        d.severity = sev;
                    }
                }
                diagnostics.extend(diags);
            }
        }

        // Phase 2: check_module
        for rule in self.registry.enabled_rules() {
            let mut diags = rule.check_module(&ctx);
            for d in &mut diags {
                if let Some(sev) = self.registry.configured_severity(&rule.meta().name) {
                    d.severity = sev;
                }
            }
            diagnostics.extend(diags);
        }

        // Collect hazards from diagnostics (for non-test source files)
        // Only Error-severity diagnostics become hazards — Warning-level findings
        // (e.g., call/new expressions) don't trigger hazard-reachability errors.
        if !ctx.is_test_file {
            for d in &diagnostics {
                if d.severity != Severity::Error {
                    continue;
                }
                hazards.push(Hazard {
                    rule_name: d.rule_name.clone(),
                    category: HazardCategory::MutableState, // simplified for now
                    confidence: crate::rule::Confidence::Definite,
                    span: d.span,
                    message: d.message.clone(),
                });
            }
        }

        let test_context = if ctx.is_test_file {
            Some(TestContextSummary {
                file_path: file_path.to_path_buf(),
                mocks: ctx.mocks.clone(),
                safe_signals: ctx.safe_signals.clone(),
                framework: ctx.framework,
            })
        } else {
            None
        };

        Some(FileAnalysisResult {
            file_path: file_path.to_path_buf(),
            diagnostics,
            hazards,
            imports: ctx.imports,
            test_context,
        })
    }

    /// Build the module graph from analyzed files.
    fn build_graph(
        &self,
        results: &[FileAnalysisResult],
        all_imports: &HashMap<PathBuf, Vec<crate::engine::context::ImportInfo>>,
    ) -> ModuleGraph {
        let mut graph = ModuleGraph::new();

        // Create resolver
        let resolver = self.create_resolver();

        // Add all analyzed files as nodes
        for result in results {
            graph.add_node(
                result.file_path.clone(),
                result.test_context.is_some(),
            );
            graph.set_hazards(&result.file_path, result.hazards.clone());
        }

        // Resolve imports and add edges
        for (file_path, imports) in all_imports {
            let dir = file_path.parent().unwrap_or(Path::new("."));
            for import in imports {
                // Skip node builtins and bare specifiers from node_modules
                if import.source.starts_with("node:")
                    || (!import.source.starts_with('.')
                        && !import.source.starts_with('/')
                        && !import.source.starts_with('@'))
                {
                    // Could be a bare specifier, try to resolve but skip if it's in node_modules
                    if let Ok(resolved) = resolver.resolve(dir, &import.source) {
                        let resolved_path = resolved.path().to_path_buf();
                        if resolved_path.to_string_lossy().contains("node_modules") {
                            continue;
                        }
                        graph.add_node(resolved_path.clone(), false);
                        let kind = if import.is_side_effect {
                            EdgeKind::SideEffectImport
                        } else {
                            EdgeKind::StaticImport
                        };
                        graph.add_edge(
                            file_path.clone(),
                            resolved_path,
                            kind,
                            import.is_type_only,
                        );
                    }
                    continue;
                }

                match resolver.resolve(dir, &import.source) {
                    Ok(resolved) => {
                        let resolved_path = resolved.path().to_path_buf();
                        if resolved_path.to_string_lossy().contains("node_modules") {
                            continue;
                        }
                        graph.add_node(resolved_path.clone(), false);
                        let kind = if import.is_side_effect {
                            EdgeKind::SideEffectImport
                        } else {
                            EdgeKind::StaticImport
                        };
                        graph.add_edge(
                            file_path.clone(),
                            resolved_path,
                            kind,
                            import.is_type_only,
                        );
                    }
                    Err(_) => {
                        // Module not found — skip silently
                    }
                }
            }
        }

        graph
    }

    fn create_resolver(&self) -> Resolver {
        let mut options = ResolveOptions {
            extensions: vec![
                ".ts".into(),
                ".tsx".into(),
                ".js".into(),
                ".jsx".into(),
                ".mjs".into(),
                ".cjs".into(),
            ],
            main_fields: vec!["module".into(), "main".into()],
            condition_names: vec!["import".into(), "require".into(), "default".into()],
            ..Default::default()
        };

        // Use tsconfig if available
        if let Some(ref tsconfig_path) = self.config.tsconfig_path {
            options.tsconfig = Some(TsconfigOptions {
                config_file: tsconfig_path.clone().into(),
                references: TsconfigReferences::Auto,
            });
        }

        Resolver::new(options)
    }

    /// Build the global mock registry from all test contexts.
    fn build_mock_registry(
        &self,
        test_contexts: &HashMap<PathBuf, TestContextSummary>,
    ) -> HashMap<PathBuf, Vec<(PathBuf, MockKind)>> {
        let mut registry: HashMap<PathBuf, Vec<(PathBuf, MockKind)>> = HashMap::new();

        for (test_file, ctx) in test_contexts {
            for mock in &ctx.mocks {
                if let Some(ref resolved) = mock.resolved_path {
                    registry
                        .entry(resolved.clone())
                        .or_default()
                        .push((test_file.clone(), mock.kind.clone()));
                }
            }
        }

        registry
    }

    /// Hazard reachability: for each test file, find hazardous modules in the effective subgraph.
    fn run_hazard_reachability(&self, ctx: &GraphContext, progress: &dyn Progress) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();

        for (test_file, test_ctx) in &ctx.test_contexts {
            let effective = ctx.graph.effective_subgraph(test_file, &test_ctx.mocks);
            progress.reachability_step();

            for module_path in &effective {
                if module_path == test_file {
                    continue;
                }

                // Skip allowed modules
                if self.config.is_allowed(module_path) {
                    continue;
                }

                // Check if module has hazards
                if let Some(hazards) = ctx.module_hazards.get(module_path) {
                    if hazards.is_empty() {
                        continue;
                    }

                    let module_name = module_path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("unknown");

                    let hazard_summary: Vec<String> = hazards
                        .iter()
                        .take(3)
                        .map(|h| h.message.clone())
                        .collect();

                    let message = format!(
                        "Unmocked hazardous module `{}`: {}",
                        module_name,
                        hazard_summary.join("; ")
                    );

                    diagnostics.push(Diagnostic {
                        rule_name: "hazard-reachability".to_string(),
                        severity: Severity::Error,
                        message,
                        file_path: test_file.clone(),
                        span: oxc_span::Span::default(),
                        help: Some(format!(
                            "This module has {} hazard(s). Mock it to isolate your tests.",
                            hazards.len()
                        )),
                        fix: Some(Fix {
                            text: module_path.to_string_lossy().to_string(),
                            span: oxc_span::Span::default(),
                        }),
                    });
                }
            }
        }

        diagnostics
    }

    /// Run Phase 3 graph-level rules.
    fn run_graph_rules(&self, ctx: &GraphContext) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();

        for rule in self.registry.enabled_rules() {
            let mut diags = rule.check_graph(ctx);
            for d in &mut diags {
                if let Some(sev) = self.registry.configured_severity(&rule.meta().name) {
                    d.severity = sev;
                }
            }
            diagnostics.extend(diags);
        }

        diagnostics
    }
}

/// Result from analyzing a single file.
struct FileAnalysisResult {
    file_path: PathBuf,
    diagnostics: Vec<Diagnostic>,
    hazards: Vec<Hazard>,
    imports: Vec<crate::engine::context::ImportInfo>,
    test_context: Option<TestContextSummary>,
}
