//! A transaction guard whose `Drop` rolls back.
//!
//! The raw calls live in [`crate::tx`] (generated). This wraps them so a
//! transaction that goes out of scope without an explicit
//! [`Transaction::commit`] — an early `?`, a panic-free error path — is rolled
//! back rather than left open on the server for the rest of the execution.
//!
//! ```ignore
//! let tx = Transaction::begin()?;
//! raisin_sdk::tx::create(tx.id(), "content", "/people", &person)?;
//! tx.commit()?;   // without this line, Drop rolls back
//! ```

use crate::error::Result;
use crate::tx;

/// An open transaction. Rolls back on drop unless committed.
#[derive(Debug)]
pub struct Transaction {
    id: String,
    settled: bool,
}

impl Transaction {
    /// Begin a transaction.
    pub fn begin() -> Result<Self> {
        Ok(Self {
            id: tx::begin()?,
            settled: false,
        })
    }

    /// The transaction id every `raisin_sdk::tx::*` call takes.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Attribute the transaction's writes to `actor`.
    pub fn set_actor(&self, actor: &str) -> Result<()> {
        tx::set_actor(&self.id, actor)
    }

    /// Attach a commit message.
    pub fn set_message(&self, message: &str) -> Result<()> {
        tx::set_message(&self.id, message)
    }

    /// Commit. Consumes the guard, so `Drop` will not roll back.
    pub fn commit(mut self) -> Result<()> {
        self.settled = true;
        tx::commit(&self.id)
    }

    /// Roll back explicitly. Consumes the guard.
    pub fn rollback(mut self) -> Result<()> {
        self.settled = true;
        tx::rollback(&self.id)
    }
}

impl Drop for Transaction {
    fn drop(&mut self) {
        if self.settled {
            return;
        }
        // Best effort: a Drop cannot answer with an error, and a failed
        // rollback is still better reported than swallowed silently.
        if let Err(e) = tx::rollback(&self.id) {
            crate::log::warn(format!(
                "rolling back uncommitted transaction {} failed: {e}",
                self.id
            ));
        }
    }
}
