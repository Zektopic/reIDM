//! TEMP: replaced by Manager::revision() when the engine branch lands.
//!
//! The service's conditional `GET /api/downloads` and the row cache behind
//! `GET /api/downloads/rows` are written against `Manager::revision()`, a number the engine
//! changes whenever anything visible in the list changes, progress included. Until the engine
//! provides it, this derives one from the list itself: a hash of everything `list()` returns.
//! That is correct (two equal values mean an identical list) but not cheap, since it clones and
//! serialises the whole list; the real counter costs one atomic load.
//!
//! Callers only ever compare revisions for equality, never for order, so a hash is a faithful
//! stand-in for a counter. When the engine's inherent `Manager::revision` exists it takes
//! precedence over this trait method at every call site; delete this file and its `mod` line.

use reidm_core::Manager;
use std::hash::Hasher;

pub trait Revision {
    fn revision(&self) -> u64;
}

/// Feeds serialised bytes straight into a hasher, so no copy of the JSON is ever built.
struct HashWriter<H: Hasher>(H);

impl<H: Hasher> std::io::Write for HashWriter<H> {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.write(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Revision for Manager {
    fn revision(&self) -> u64 {
        let mut w = HashWriter(std::collections::hash_map::DefaultHasher::new());
        let _ = serde_json::to_writer(&mut w, &self.list());
        w.0.finish()
    }
}
