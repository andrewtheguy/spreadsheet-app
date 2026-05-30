use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use sheet_core::CsvTable;

// Backend-held store of loaded tables, keyed by an opaque id handed to the frontend.
// Lets `filter_csv` / `compare_csv` / `common_columns` reference already-loaded tables by
// id instead of re-shipping full table payloads across the IPC boundary on every recompute
// (column/mode/case-insensitive toggle). Lives in the Tauri layer, not `sheet-core`, so the
// core stays pure logic over `&CsvTable`.
#[derive(Default)]
pub struct TableStore {
    inner: Mutex<TableStoreInner>,
}

#[derive(Default)]
struct TableStoreInner {
    next_id: u64,
    tables: HashMap<u64, Arc<CsvTable>>,
}

impl TableStore {
    // Inserts `table`, evicting `replace` (the superseded id, e.g. a side being reloaded) if
    // given, and returns the new id. The eviction keeps the store from holding more than the
    // live tables without a separate free command.
    pub fn insert(&self, table: CsvTable, replace: Option<u64>) -> u64 {
        let mut inner = self.inner.lock().unwrap();
        if let Some(old) = replace {
            inner.tables.remove(&old);
        }
        let id = inner.next_id;
        inner.next_id += 1;
        inner.tables.insert(id, Arc::new(table));
        id
    }

    // An `Arc` handle to the table for `id`, or an error if it's unknown (so a stale call
    // referencing an evicted table fails cleanly instead of panicking). Cloning the `Arc`
    // lets the caller release the lock before handing the table to `sheet-core`.
    pub fn get(&self, id: u64) -> Result<Arc<CsvTable>, String> {
        self.inner
            .lock()
            .unwrap()
            .tables
            .get(&id)
            .cloned()
            .ok_or_else(|| format!("no table loaded for id {id}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(tag: &str) -> CsvTable {
        CsvTable {
            headers: vec![tag.to_string()],
            rows: vec![vec![tag.to_string()]],
        }
    }

    #[test]
    fn insert_returns_increasing_ids() {
        let store = TableStore::default();
        let a = store.insert(table("a"), None);
        let b = store.insert(table("b"), None);
        assert_ne!(a, b);
        assert_eq!(*store.get(a).unwrap(), table("a"));
        assert_eq!(*store.get(b).unwrap(), table("b"));
    }

    #[test]
    fn get_unknown_id_errors() {
        let store = TableStore::default();
        assert!(store.get(404).is_err());
    }

    #[test]
    fn replace_evicts_the_old_id() {
        let store = TableStore::default();
        let old = store.insert(table("old"), None);
        let new = store.insert(table("new"), Some(old));
        assert!(store.get(old).is_err());
        assert_eq!(*store.get(new).unwrap(), table("new"));
    }
}
