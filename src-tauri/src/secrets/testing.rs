use super::*;

/// In-memory backend that counts reads (to test the cache).
#[derive(Default, Clone)]
pub struct MemoryBackend {
    pub data: Arc<Mutex<HashMap<String, String>>>,
    pub reads: Arc<Mutex<usize>>,
}

impl SecretBackend for MemoryBackend {
    fn read(&self, account: &str) -> Result<Option<String>, String> {
        *self.reads.lock().unwrap() += 1;
        Ok(self.data.lock().unwrap().get(account).cloned())
    }
    fn write(&self, account: &str, value: &str) -> Result<(), String> {
        self.data
            .lock()
            .unwrap()
            .insert(account.into(), value.into());
        Ok(())
    }
    fn delete(&self, account: &str) -> Result<(), String> {
        self.data.lock().unwrap().remove(account);
        Ok(())
    }
}
