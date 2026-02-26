use isofence::engine::context::{ModuleContext, TestFramework};
use isofence::engine::parser::{extract_imports, parse_source};
use isofence::rule::Rule;
use isofence::rules;
use oxc_allocator::Allocator;
use std::path::PathBuf;

// ---- Helper ----

fn check_rule(source: &str, rule: &dyn Rule) -> Vec<String> {
    let allocator = Allocator::default();
    let file_path = PathBuf::from("test_module.ts");
    let result = parse_source(&allocator, source, &file_path);
    assert!(!result.panicked, "Parse failed for source:\n{source}");

    let imports = extract_imports(&result.program);
    let mut ctx = ModuleContext::new(file_path, source.to_string(), TestFramework::Vitest);
    ctx.imports = imports;

    let mut diagnostics = Vec::new();
    for stmt in &result.program.body {
        diagnostics.extend(rule.check_module_item(stmt, &ctx));
    }
    diagnostics.extend(rule.check_module(&ctx));

    diagnostics.into_iter().map(|d| d.message).collect()
}

fn check_all_rules(source: &str) -> Vec<String> {
    let allocator = Allocator::default();
    let file_path = PathBuf::from("test_module.ts");
    let result = parse_source(&allocator, source, &file_path);
    assert!(!result.panicked, "Parse failed");

    let imports = extract_imports(&result.program);
    let mut ctx = ModuleContext::new(file_path, source.to_string(), TestFramework::Vitest);
    ctx.imports = imports;

    let all_rules = rules::all_builtin_rules();
    let mut diagnostics = Vec::new();
    for rule in &all_rules {
        for stmt in &result.program.body {
            diagnostics.extend(rule.check_module_item(stmt, &ctx));
        }
        diagnostics.extend(rule.check_module(&ctx));
    }

    diagnostics.into_iter().map(|d| d.message).collect()
}

// ---- mutable-module-var ----

mod mutable_module_var {
    use super::*;
    use isofence::rules::mutable_module_var::MutableModuleVar;

    fn check(source: &str) -> Vec<String> {
        check_rule(source, &MutableModuleVar)
    }

    #[test]
    fn detects_let_at_module_scope() {
        let msgs = check("let counter = 0;");
        assert_eq!(msgs.len(), 1);
        assert!(msgs[0].contains("let counter"));
    }

    #[test]
    fn detects_var_at_module_scope() {
        let msgs = check("var flag = true;");
        assert_eq!(msgs.len(), 1);
        assert!(msgs[0].contains("var flag"));
    }

    #[test]
    fn detects_exported_let() {
        let msgs = check("export let value = 42;");
        assert_eq!(msgs.len(), 1);
        assert!(msgs[0].contains("let value"));
    }

    #[test]
    fn ignores_const() {
        assert!(check("const x = 1;").is_empty());
    }

    #[test]
    fn ignores_function_declaration() {
        assert!(check("function foo() { let x = 1; }").is_empty());
    }

    #[test]
    fn detects_multiple_let_declarations() {
        let msgs = check("let a = 1, b = 2;");
        assert_eq!(msgs.len(), 2);
    }
}

// ---- mutable-const-init ----

mod mutable_const_init {
    use super::*;
    use isofence::rules::mutable_const_init::MutableConstInit;

    fn check(source: &str) -> Vec<String> {
        check_rule(source, &MutableConstInit)
    }

    #[test]
    fn detects_const_object() {
        let msgs = check("const state = { count: 0 };");
        assert_eq!(msgs.len(), 1);
        assert!(msgs[0].contains("object literal"));
    }

    #[test]
    fn detects_const_array() {
        let msgs = check("const items: string[] = [];");
        assert_eq!(msgs.len(), 1);
        assert!(msgs[0].contains("array literal"));
    }

    #[test]
    fn detects_const_new() {
        let msgs = check("const client = new HttpClient();");
        assert_eq!(msgs.len(), 1);
        assert!(msgs[0].contains("constructor"));
    }

    #[test]
    fn detects_const_function_call() {
        let msgs = check("const logger = createLogger();");
        assert_eq!(msgs.len(), 1);
        assert!(msgs[0].contains("function call"));
    }

    #[test]
    fn detects_const_map() {
        let msgs = check("const cache = new Map();");
        assert_eq!(msgs.len(), 1);
        assert!(msgs[0].contains("collection"));
    }

    #[test]
    fn detects_const_set() {
        let msgs = check("const seen = new Set();");
        assert_eq!(msgs.len(), 1);
        assert!(msgs[0].contains("collection"));
    }

    #[test]
    fn detects_stateful_regex() {
        let msgs = check("const RE = /pattern/g;");
        assert_eq!(msgs.len(), 1);
        assert!(msgs[0].contains("RegExp"));
    }

    #[test]
    fn ignores_const_primitive() {
        assert!(check("const MAX = 3;").is_empty());
    }

    #[test]
    fn ignores_const_string() {
        assert!(check(r#"const URL = "https://api.example.com";"#).is_empty());
    }

    #[test]
    fn ignores_const_boolean() {
        assert!(check("const ENABLED = true;").is_empty());
    }

    #[test]
    fn ignores_const_null() {
        assert!(check("const NOTHING = null;").is_empty());
    }

    #[test]
    fn ignores_object_freeze() {
        assert!(check("const CONFIG = Object.freeze({ a: 1 });").is_empty());
    }

    #[test]
    fn detects_exported_mutable_const() {
        let msgs = check("export const cache = new Map();");
        assert_eq!(msgs.len(), 1);
    }

    #[test]
    fn ignores_const_undefined() {
        assert!(check("const x = undefined;").is_empty());
    }

    #[test]
    fn ignores_template_literal() {
        assert!(check("const msg = `hello world`;").is_empty());
    }

    #[test]
    fn detects_regex_with_sticky_flag() {
        let msgs = check("const RE = /pat/y;");
        assert_eq!(msgs.len(), 1);
        assert!(msgs[0].contains("RegExp"));
    }

    #[test]
    fn ignores_regex_without_stateful_flags() {
        // Non-stateful regex is still a RegExpLiteral but not g/y
        // It's treated as a primitive-like value
        assert!(check("const RE = /pattern/i;").is_empty());
    }
}

// ---- top-level-call ----

mod top_level_call {
    use super::*;
    use isofence::rules::top_level_call::TopLevelCall;

    fn check(source: &str) -> Vec<String> {
        check_rule(source, &TopLevelCall)
    }

    #[test]
    fn detects_top_level_call() {
        let msgs = check("initializeApp();");
        assert_eq!(msgs.len(), 1);
        assert!(msgs[0].contains("initializeApp"));
    }

    #[test]
    fn detects_member_call() {
        let msgs = check("console.log('loaded');");
        assert_eq!(msgs.len(), 1);
        assert!(msgs[0].contains("console.log"));
    }

    #[test]
    fn ignores_call_inside_function() {
        assert!(check("function foo() { bar(); }").is_empty());
    }

    #[test]
    fn ignores_variable_declaration_with_call() {
        assert!(check("const x = foo();").is_empty());
    }
}

// ---- global-mutation ----

mod global_mutation {
    use super::*;
    use isofence::rules::global_mutation::GlobalMutation;

    fn check(source: &str) -> Vec<String> {
        check_rule(source, &GlobalMutation)
    }

    #[test]
    fn detects_globalthis_assignment() {
        let msgs = check("globalThis.myApp = {};");
        assert_eq!(msgs.len(), 1);
        assert!(msgs[0].contains("globalThis.myApp"));
    }

    #[test]
    fn detects_process_env_assignment() {
        let msgs = check("process.env.NODE_ENV = 'test';");
        assert_eq!(msgs.len(), 1);
        assert!(msgs[0].contains("process.env"));
    }

    #[test]
    fn detects_window_assignment() {
        let msgs = check("window.DEBUG = true;");
        assert_eq!(msgs.len(), 1);
        assert!(msgs[0].contains("window.DEBUG"));
    }

    #[test]
    fn ignores_non_global_assignment() {
        // Assignment to a local var's property — not a recognized global
        let msgs = check("myObj.foo = 1;");
        assert!(msgs.is_empty());
    }
}

// ---- event-subscription ----

mod event_subscription {
    use super::*;
    use isofence::rules::event_subscription::EventSubscription;

    fn check(source: &str) -> Vec<String> {
        check_rule(source, &EventSubscription)
    }

    #[test]
    fn detects_on_listener() {
        let msgs = check("emitter.on('data', handler);");
        assert_eq!(msgs.len(), 1);
        assert!(msgs[0].contains("emitter.on"));
    }

    #[test]
    fn detects_addeventlistener() {
        let msgs = check("document.addEventListener('click', handler);");
        assert_eq!(msgs.len(), 1);
        assert!(msgs[0].contains("addEventListener"));
    }

    #[test]
    fn detects_subscribe() {
        let msgs = check("observable.subscribe(handler);");
        assert_eq!(msgs.len(), 1);
        assert!(msgs[0].contains("subscribe"));
    }

    #[test]
    fn ignores_non_event_call() {
        assert!(check("foo.bar();").is_empty());
    }
}

// ---- top-level-await ----

mod top_level_await {
    use super::*;
    use isofence::rules::top_level_await::TopLevelAwait;

    fn check(source: &str) -> Vec<String> {
        check_rule(source, &TopLevelAwait)
    }

    #[test]
    fn detects_await_expression_statement() {
        let msgs = check("await initSchema();");
        assert_eq!(msgs.len(), 1);
        assert!(msgs[0].contains("await"));
    }

    #[test]
    fn detects_const_await() {
        let msgs = check("const db = await connect();");
        assert_eq!(msgs.len(), 1);
        assert!(msgs[0].contains("await"));
    }

    #[test]
    fn ignores_await_inside_function() {
        assert!(check("async function foo() { await bar(); }").is_empty());
    }
}

// ---- iife ----

mod iife {
    use super::*;
    use isofence::rules::iife::Iife;

    fn check(source: &str) -> Vec<String> {
        check_rule(source, &Iife)
    }

    #[test]
    fn detects_function_iife() {
        let msgs = check("(function() { })();");
        assert_eq!(msgs.len(), 1);
        assert!(msgs[0].contains("IIFE"));
    }

    #[test]
    fn detects_arrow_iife() {
        let msgs = check("(() => { })();");
        assert_eq!(msgs.len(), 1);
        assert!(msgs[0].contains("IIFE"));
    }

    #[test]
    fn ignores_normal_call() {
        assert!(check("foo();").is_empty());
    }
}

// ---- prototype-mutation ----

mod prototype_mutation {
    use super::*;
    use isofence::rules::prototype_mutation::PrototypeMutation;

    fn check(source: &str) -> Vec<String> {
        check_rule(source, &PrototypeMutation)
    }

    #[test]
    fn detects_prototype_assignment() {
        let msgs = check("Array.prototype.customMethod = function() {};");
        assert_eq!(msgs.len(), 1);
        assert!(msgs[0].contains("prototype"));
    }

    #[test]
    fn detects_string_prototype() {
        let msgs = check("String.prototype.toTitle = function() {};");
        assert_eq!(msgs.len(), 1);
    }

    #[test]
    fn ignores_non_prototype_assignment() {
        assert!(check("obj.foo = 1;").is_empty());
    }
}

// ---- side-effect-import ----

mod side_effect_import {
    use super::*;
    use isofence::rules::side_effect_import::SideEffectImport;

    fn check(source: &str) -> Vec<String> {
        check_rule(source, &SideEffectImport)
    }

    #[test]
    fn detects_side_effect_import() {
        let msgs = check("import './setup';");
        assert_eq!(msgs.len(), 1);
        assert!(msgs[0].contains("./setup"));
    }

    #[test]
    fn detects_multiple() {
        let msgs = check("import './setup';\nimport './polyfill';");
        assert_eq!(msgs.len(), 2);
    }

    #[test]
    fn ignores_named_import() {
        assert!(check("import { foo } from './bar';").is_empty());
    }

    #[test]
    fn ignores_type_import() {
        assert!(check("import type { Config } from './types';").is_empty());
    }
}

// ---- static-class-field ----

mod static_class_field {
    use super::*;
    use isofence::rules::static_class_field::StaticClassField;

    fn check(source: &str) -> Vec<String> {
        check_rule(source, &StaticClassField)
    }

    #[test]
    fn detects_static_mutable_field() {
        let msgs = check("class Foo { static instances = new Map(); }");
        assert_eq!(msgs.len(), 1);
        assert!(msgs[0].contains("instances"));
    }

    #[test]
    fn detects_static_object_field() {
        let msgs = check("class Foo { static config = { debug: false }; }");
        assert_eq!(msgs.len(), 1);
    }

    #[test]
    fn ignores_static_primitive_field() {
        assert!(check("class Foo { static VERSION = '1.0'; }").is_empty());
    }

    #[test]
    fn ignores_non_static_field() {
        assert!(check("class Foo { instances = new Map(); }").is_empty());
    }

    #[test]
    fn detects_exported_class() {
        let msgs = check("export class Registry { static data = []; }");
        assert_eq!(msgs.len(), 1);
    }
}

// ---- safe module ----

mod safe_module {
    use super::*;

    #[test]
    fn safe_module_no_diagnostics() {
        let source = r#"
export type User = { id: string; name: string };
export interface Config { apiUrl: string }
export const MAX_RETRIES = 3;
export const API_URL = "https://api.example.com";
export function add(a: number, b: number): number { return a + b; }
export class Formatter {
  format(value: string): string { return value.trim(); }
}
"#;
        let msgs = check_all_rules(source);
        assert!(msgs.is_empty(), "Safe module should have zero diagnostics, got: {msgs:?}");
    }

    #[test]
    fn pure_type_module_no_diagnostics() {
        let source = r#"
export type ID = string;
export type Name = string;
export interface User { id: ID; name: Name }
"#;
        assert!(check_all_rules(source).is_empty());
    }

    #[test]
    fn const_primitives_no_diagnostics() {
        let source = r#"
const A = 1;
const B = "hello";
const C = true;
const D = null;
const E = undefined;
"#;
        assert!(check_all_rules(source).is_empty());
    }
}

// ---- as const detection ----

mod as_const {
    use super::*;

    #[test]
    fn ignores_as_const_object() {
        assert!(check_all_rules("const CONFIG = { a: 1, b: 2 } as const;").is_empty());
    }

    #[test]
    fn ignores_as_const_array() {
        assert!(check_all_rules("const ITEMS = [1, 2, 3] as const;").is_empty());
    }

    #[test]
    fn ignores_object_freeze() {
        assert!(check_all_rules("const FROZEN = Object.freeze({ a: 1 });").is_empty());
    }
}
