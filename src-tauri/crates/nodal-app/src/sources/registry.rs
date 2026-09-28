//! Registry of provider factories (Linear today; more adapters register the same way).

use std::collections::HashMap;
use std::sync::Arc;

use nodal_domain::ports::ProviderFactory;

pub struct ProviderRegistry {
    factories: HashMap<&'static str, Arc<dyn ProviderFactory>>,
}

impl ProviderRegistry {
    pub fn new(factories: Vec<Arc<dyn ProviderFactory>>) -> Self {
        Self {
            factories: factories.into_iter().map(|f| (f.name(), f)).collect(),
        }
    }

    pub fn names(&self) -> Vec<&'static str> {
        self.factories.keys().copied().collect()
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn ProviderFactory>> {
        self.factories.get(name).cloned()
    }
}
