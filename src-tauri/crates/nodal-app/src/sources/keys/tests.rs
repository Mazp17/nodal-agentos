use std::sync::Arc;

use super::testing::MemoryBackend;
use super::*;
use crate::testutil::block_on;

#[test]
fn account_names() {
    assert_eq!(account_for("linear").unwrap(), "linear-api-key");
    assert_eq!(account_for("azure_devops").unwrap(), "azure_devops-api-key");
    assert!(account_for("").is_err());
    assert!(account_for("Linear").is_err());
    assert!(account_for("a/b").is_err());
}

#[test]
fn get_set_delete_with_cache() {
    block_on(async {
        let mem = MemoryBackend::default();
        mem.data
            .lock()
            .unwrap()
            .insert("linear-api-key".into(), "  lin_abc \n".into());
        let s = Secrets::new(Arc::new(mem.clone()));

        assert_eq!(s.get("linear").await.unwrap().as_deref(), Some("lin_abc"));
        assert_eq!(s.get("linear").await.unwrap().as_deref(), Some("lin_abc"));
        assert_eq!(
            *mem.reads.lock().unwrap(),
            1,
            "the second read comes from the cache"
        );

        assert_eq!(s.get("asana").await.unwrap(), None);
        s.set("asana", " as_1 ").await.unwrap();
        assert_eq!(s.get("asana").await.unwrap().as_deref(), Some("as_1"));
        assert_eq!(
            mem.data
                .lock()
                .unwrap()
                .get("asana-api-key")
                .map(String::as_str),
            Some("as_1")
        );

        s.delete("linear").await.unwrap();
        assert_eq!(s.get("linear").await.unwrap(), None);
        assert!(!mem.data.lock().unwrap().contains_key("linear-api-key"));

        s.set("asana", "   ").await.unwrap();
        assert_eq!(s.get("asana").await.unwrap(), None);
        assert!(s.get("Bad/Name").await.is_err());
    });
}
