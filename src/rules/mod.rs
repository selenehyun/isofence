pub mod event_subscription;
pub mod global_mutation;
pub mod iife;
pub mod mock_consensus;
pub mod mutable_const_init;
pub mod mutable_module_var;
pub mod prototype_mutation;
pub mod side_effect_import;
pub mod static_class_field;
pub mod top_level_await;
pub mod top_level_call;

use crate::rule::Rule;

/// Create all built-in rules.
pub fn all_builtin_rules() -> Vec<Box<dyn Rule>> {
    vec![
        Box::new(mutable_module_var::MutableModuleVar),
        Box::new(mutable_const_init::MutableConstInit),
        Box::new(top_level_call::TopLevelCall),
        Box::new(global_mutation::GlobalMutation),
        Box::new(event_subscription::EventSubscription),
        Box::new(top_level_await::TopLevelAwait),
        Box::new(iife::Iife),
        Box::new(prototype_mutation::PrototypeMutation),
        Box::new(side_effect_import::SideEffectImport),
        Box::new(static_class_field::StaticClassField),
        Box::new(mock_consensus::MockConsensus),
    ]
}
