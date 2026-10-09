//! `ApprovalRegistry`: holds registered `ApprovalSubject` and `ApprovalPolicy` pairs.

use std::collections::HashMap;
use std::sync::Arc;

use crate::domain::policy::ApprovalPolicy;
use crate::domain::subject::ApprovalSubject;

/// A registered pair of subject and policy.
pub type RegistryEntry = (Arc<dyn ApprovalSubject>, Arc<dyn ApprovalPolicy>);

/// Registry of approval subjects and their corresponding policies, keyed by `type_name()`.
#[derive(Default)]
pub struct ApprovalRegistry {
    entries: HashMap<String, RegistryEntry>,
}

impl ApprovalRegistry {
    /// Creates an empty registry.
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    /// Registers an `ApprovalSubject` and `ApprovalPolicy` pair.
    pub fn register(&mut self, subject: Arc<dyn ApprovalSubject>, policy: Arc<dyn ApprovalPolicy>) {
        let type_name = subject.type_name().to_string();
        self.entries.insert(type_name, (subject, policy));
    }

    /// Looks up a registered subject and policy pair by type name.
    pub fn get(&self, type_name: &str) -> Option<RegistryEntry> {
        self.entries.get(type_name).cloned()
    }
}
