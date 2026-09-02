/// Which caller asked `rollback_one_block` to rewind (INC-I-204 M4.2).
///
/// Exactly the two production call sites. `ReorgPlan` / `WedgeEscape` are
/// reserved for M6, when `execute_reorg` and `wedge_escape` consolidate onto
/// this door; adding them here would be never-constructed dead code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RollbackAuthority {
    /// RecoveryCoordinator `ShallowRollback` (`periodic.rs`). Unguarded.
    CoordinatorApproved { depth: u32 },
    /// The production poison arm (`production/poison.rs`). Authorised only when
    /// the local tip IS the block that failed to apply.
    ProductionSelfApply { failed_height: u64 },
}
