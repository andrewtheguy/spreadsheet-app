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

// Holds exactly the *latest* derived result (a filter or compare result), keyed by an opaque
// id handed to the frontend. Unlike `TableStore`, there is only ever one entry: each `set`
// overwrites it, so a superseded recompute can't leak — the frontend never needs to evict the
// previous id. `get` rejects any id that isn't the current one, so a stale async call (a sort
// or export for a result that's since been recomputed) fails cleanly instead of acting on the
// wrong data. Generic so it can hold a filtered `CsvTable` or a `ComparisonResult`.
pub struct LatestSlot<T> {
    inner: Mutex<LatestSlotInner<T>>,
}

struct LatestSlotInner<T> {
    next_id: u64,
    current: Option<(u64, Arc<T>)>,
}

// Derived rather than `#[derive(Default)]` because that would wrongly require `T: Default`.
impl<T> Default for LatestSlot<T> {
    fn default() -> Self {
        LatestSlot {
            inner: Mutex::new(LatestSlotInner {
                next_id: 0,
                current: None,
            }),
        }
    }
}

impl<T> LatestSlot<T> {
    // Replaces the held value with `value` and returns its new id. The previous value (if any)
    // is dropped, keeping at most one result in memory.
    pub fn set(&self, value: T) -> u64 {
        let mut inner = self.inner.lock().unwrap();
        let id = inner.next_id;
        inner.next_id += 1;
        inner.current = Some((id, Arc::new(value)));
        id
    }

    // An `Arc` handle to the held value when `id` matches the current one, else an error (the
    // result was superseded by a newer recompute, or nothing is loaded). Cloning the `Arc` lets
    // the caller release the lock before handing the value to `sheet-core`.
    pub fn get(&self, id: u64) -> Result<Arc<T>, String> {
        match &self.inner.lock().unwrap().current {
            Some((cur, value)) if *cur == id => Ok(value.clone()),
            _ => Err(format!("no current result for id {id}")),
        }
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

    #[test]
    fn latest_slot_set_returns_increasing_ids_and_overwrites() {
        let slot = LatestSlot::default();
        let a = slot.set(table("a"));
        let b = slot.set(table("b"));
        assert_ne!(a, b);
        // The old id is superseded; only the latest is retrievable.
        assert!(slot.get(a).is_err());
        assert_eq!(*slot.get(b).unwrap(), table("b"));
    }

    #[test]
    fn latest_slot_get_unknown_or_empty_errors() {
        let slot: LatestSlot<CsvTable> = LatestSlot::default();
        assert!(slot.get(0).is_err());
        let id = slot.set(table("x"));
        assert!(slot.get(id + 1).is_err());
    }
}
