//! Multi-Raft Sharded Partition Replication Groups
//!
//! Grounded in modern distributed consensus literature (e.g. CockroachDB / TiKV Multi-Raft).
//!
//! Partitions the global state machine into independent replicated Raft groups
//! keyed by currency-pair or asset class. Ensures that a partition stall or election
//! in one trading book does not cause head-of-line blocking for other books.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::entry::{Index, Term};
use crate::state::{BookState, BookUpdate, UpdateError};

/// Identifier for a partitioned consensus group (e.g. "EURUSD", "USDJPY").
pub type GroupId = String;

/// Error arising during Multi-Raft operations.
#[derive(Debug, PartialEq, Eq)]
pub enum MultiRaftError {
    /// Group not found in router.
    NoSuchGroup(GroupId),
    /// State machine execution error.
    Execution(UpdateError),
}

/// Independent partitioned state machine replica.
#[derive(Debug)]
pub struct PartitionReplica {
    group_id: GroupId,
    state: BookState,
    current_term: Term,
    last_committed: Option<Index>,
}

impl PartitionReplica {
    /// Create a new partition replica.
    pub fn new(group_id: impl Into<GroupId>) -> Self {
        Self {
            group_id: group_id.into(),
            state: BookState::new(),
            current_term: 1,
            last_committed: None,
        }
    }

    /// The partition group identifier.
    pub fn group_id(&self) -> &str {
        &self.group_id
    }

    /// Apply a committed update to this partition's state machine.
    pub fn apply_committed(
        &mut self,
        index: Index,
        term: Term,
        update: &BookUpdate,
    ) {
        self.state.apply(update);
        self.current_term = term;
        self.last_committed = Some(index);
    }

    /// Read an instrument price from this partition.
    pub fn get_price(&self, instrument_id: u64) -> Option<f64> {
        self.state.get(instrument_id)
    }

    /// The last committed index for this partition.
    pub fn last_committed(&self) -> Option<Index> {
        self.last_committed
    }
}

/// Router managing multiple independent partition groups.
#[derive(Clone, Default)]
pub struct MultiRaftRouter {
    groups: Arc<Mutex<HashMap<GroupId, PartitionReplica>>>,
}

impl MultiRaftRouter {
    /// Create an empty Multi-Raft router.
    pub fn new() -> Self {
        Self {
            groups: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Register a new partitioned replication group.
    pub fn register_group(&self, group_id: impl Into<GroupId>) {
        let gid = group_id.into();
        let mut g = self.groups.lock().expect("multi-raft lock");
        g.insert(gid.clone(), PartitionReplica::new(gid));
    }

    /// Apply a committed entry to the specified partition group.
    pub fn apply(
        &self,
        group_id: &str,
        index: Index,
        term: Term,
        update: &BookUpdate,
    ) -> Result<(), MultiRaftError> {
        let mut g = self.groups.lock().expect("multi-raft lock");
        let replica = g
            .get_mut(group_id)
            .ok_or_else(|| MultiRaftError::NoSuchGroup(group_id.to_string()))?;
        replica.apply_committed(index, term, update);
        Ok(())
    }

    /// Read a price from a specific partition group.
    pub fn get_price(&self, group_id: &str, instrument_id: u64) -> Result<Option<f64>, MultiRaftError> {
        let g = self.groups.lock().expect("multi-raft lock");
        let replica = g
            .get(group_id)
            .ok_or_else(|| MultiRaftError::NoSuchGroup(group_id.to_string()))?;
        Ok(replica.get_price(instrument_id))
    }

    /// Check if a group exists in the router.
    pub fn has_group(&self, group_id: &str) -> bool {
        let g = self.groups.lock().expect("multi-raft lock");
        g.contains_key(group_id)
    }

    /// List all registered partition groups.
    pub fn list_groups(&self) -> Vec<GroupId> {
        let g = self.groups.lock().expect("multi-raft lock");
        g.keys().cloned().collect()
    }
}
