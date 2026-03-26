use crypto::PublicKey;

use super::data::*;
use super::types::*;

use super::core::Transaction;

impl Transaction {
    // ==================== Maintainer Transactions ====================

    /// Create a remove maintainer transaction
    ///
    /// This transaction removes a maintainer from the auto-update system.
    /// Requires 3/5 signatures from OTHER maintainers (target cannot sign).
    ///
    /// # Arguments
    /// * `target` - Public key of the maintainer to remove
    /// * `signatures` - Signatures from at least 3 other maintainers
    /// * `reason` - Optional reason for removal (for transparency)
    pub fn new_remove_maintainer(
        target: PublicKey,
        signatures: Vec<crate::maintainer::MaintainerSignature>,
        reason: Option<String>,
    ) -> Self {
        let data = if let Some(r) = reason {
            crate::maintainer::MaintainerChangeData::with_reason(target, signatures, r)
        } else {
            crate::maintainer::MaintainerChangeData::new(target, signatures)
        };

        Self {
            version: 1,
            tx_type: TxType::RemoveMaintainer,
            inputs: Vec::new(),
            outputs: Vec::new(),
            extra_data: data.to_bytes(),
        }
    }

    /// Create an add maintainer transaction
    ///
    /// This transaction adds a new maintainer to the auto-update system.
    /// Requires 3/5 signatures from current maintainers.
    /// Target must be a registered producer.
    ///
    /// # Arguments
    /// * `target` - Public key of the producer to add as maintainer
    /// * `signatures` - Signatures from at least 3 current maintainers
    pub fn new_add_maintainer(
        target: PublicKey,
        signatures: Vec<crate::maintainer::MaintainerSignature>,
    ) -> Self {
        let data = crate::maintainer::MaintainerChangeData::new(target, signatures);

        Self {
            version: 1,
            tx_type: TxType::AddMaintainer,
            inputs: Vec::new(),
            outputs: Vec::new(),
            extra_data: data.to_bytes(),
        }
    }

    /// Check if this is a remove maintainer transaction
    pub fn is_remove_maintainer(&self) -> bool {
        self.tx_type == TxType::RemoveMaintainer
    }

    /// Check if this is an add maintainer transaction
    pub fn is_add_maintainer(&self) -> bool {
        self.tx_type == TxType::AddMaintainer
    }

    /// Check if this is any maintainer change transaction
    pub fn is_maintainer_change(&self) -> bool {
        self.is_remove_maintainer() || self.is_add_maintainer()
    }

    /// Parse maintainer change data from extra_data
    pub fn maintainer_change_data(&self) -> Option<crate::maintainer::MaintainerChangeData> {
        if !self.is_maintainer_change() {
            return None;
        }
        crate::maintainer::MaintainerChangeData::from_bytes(&self.extra_data)
    }

    /// Check if this is a delegate bond transaction
    pub fn is_delegate_bond(&self) -> bool {
        self.tx_type == TxType::DelegateBond
    }

    /// Check if this is a revoke delegation transaction
    pub fn is_revoke_delegation(&self) -> bool {
        self.tx_type == TxType::RevokeDelegation
    }

    /// Parse delegate bond data from extra_data
    pub fn delegate_bond_data(&self) -> Option<DelegateBondData> {
        if !self.is_delegate_bond() {
            return None;
        }
        DelegateBondData::from_bytes(&self.extra_data)
    }

    /// Parse revoke delegation data from extra_data
    pub fn revoke_delegation_data(&self) -> Option<RevokeDelegationData> {
        if !self.is_revoke_delegation() {
            return None;
        }
        RevokeDelegationData::from_bytes(&self.extra_data)
    }

    /// Create a new delegate bond transaction
    pub fn new_delegate_bond(data: DelegateBondData) -> Self {
        Self {
            version: 1,
            tx_type: TxType::DelegateBond,
            inputs: Vec::new(),
            outputs: Vec::new(),
            extra_data: data.to_bytes(),
        }
    }

    /// Create a new revoke delegation transaction
    pub fn new_revoke_delegation(data: RevokeDelegationData) -> Self {
        Self {
            version: 1,
            tx_type: TxType::RevokeDelegation,
            inputs: Vec::new(),
            outputs: Vec::new(),
            extra_data: data.to_bytes(),
        }
    }

    // ==================== Protocol Activation Transactions ====================

    /// Create a new protocol activation transaction
    ///
    /// Schedules new consensus rules to activate at a future epoch boundary.
    /// Requires 3/5 maintainer multisig.
    pub fn new_protocol_activation(data: crate::maintainer::ProtocolActivationData) -> Self {
        Self {
            version: 1,
            tx_type: TxType::ProtocolActivation,
            inputs: Vec::new(),
            outputs: Vec::new(),
            extra_data: data.to_bytes(),
        }
    }

    /// Check if this is a protocol activation transaction
    pub fn is_protocol_activation(&self) -> bool {
        self.tx_type == TxType::ProtocolActivation
    }

    /// Parse protocol activation data from extra_data
    pub fn protocol_activation_data(&self) -> Option<crate::maintainer::ProtocolActivationData> {
        if !self.is_protocol_activation() {
            return None;
        }
        crate::maintainer::ProtocolActivationData::from_bytes(&self.extra_data)
    }
}
