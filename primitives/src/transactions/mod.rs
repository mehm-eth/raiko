// Copyright 2023 RISC Zero, Inc.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
use core::{clone::Clone, option::Option};

use alloy_primitives::{Address, Bytes, TxHash};
use alloy_rlp::Encodable;
use serde::{Deserialize, Serialize};

use self::{
    optimism::{OptimismTxEssence, OPTIMISM_DEPOSITED_TX_TYPE},
    signature::TxSignature,
};
use crate::{keccak::keccak, transactions::ethereum::EthereumTxEssence, U256};

pub mod ethereum;
pub mod optimism;
pub mod signature;

pub type EthereumTransaction = Transaction<EthereumTxEssence>;
pub type OptimismTransaction = Transaction<OptimismTxEssence>;

/// Represents a complete transaction, encompassing its core essence and the associated
/// signature.
///
/// The `Transaction` struct encapsulates both the core details of the transaction (the
/// essence) and its cryptographic signature. The signature ensures the authenticity and
/// integrity of the transaction, confirming it was issued by the rightful sender.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Transaction<E: TxEssence> {
    /// The core details of the transaction, which includes the data that is signed.
    pub essence: E,
    /// The cryptographic signature associated with the transaction, generated by signing
    /// the transaction essence.
    pub signature: TxSignature,
}

/// Represents the core details of a [Transaction], specifically the portion that gets
/// signed.
pub trait TxEssence: Encodable + Clone {
    /// Returns the EIP-2718 transaction type or `0x00` for Legacy transactions.
    fn tx_type(&self) -> u8;
    /// Returns the gas limit set for the transaction.
    ///
    /// The gas limit represents the maximum amount of gas units that the transaction
    /// is allowed to consume. It ensures that transactions don't run indefinitely.
    fn gas_limit(&self) -> U256;
    /// Returns the recipient address of the transaction, if available.
    ///
    /// For contract creation transactions, this method returns `None` as there's no
    /// recipient address.
    fn to(&self) -> Option<Address>;
    /// Recovers the Ethereum address of the sender from the transaction's signature.
    ///
    /// This method uses the ECDSA recovery mechanism to derive the sender's public key
    /// and subsequently their Ethereum address. If the recovery is unsuccessful, an
    /// error is returned.
    fn recover_from(&self, signature: &TxSignature) -> anyhow::Result<Address>;
    /// Returns the length of the RLP-encoding payload in bytes.
    ///
    /// This method calculates the combined length of all the individual fields
    /// of the transaction when they are RLP-encoded.
    fn payload_length(&self) -> usize;
    /// Returns a reference to the transaction's call data
    fn data(&self) -> &Bytes;
}

/// Provides RLP encoding functionality for [Transaction].
impl<E: TxEssence> Encodable for Transaction<E> {
    /// Encodes the [Transaction] struct into the provided `out` buffer.
    ///
    /// The encoding process starts by prepending the EIP-2718 transaction type, if
    /// applicable. It then joins the RLP lists of the transaction essence and the
    /// signature into a single list. This approach optimizes the encoding process by
    /// reusing as much of the generated RLP code as possible.
    #[inline]
    fn encode(&self, out: &mut dyn alloy_rlp::BufMut) {
        let tx_type = self.essence.tx_type();
        // prepend the EIP-2718 transaction type for non-legacy transactions
        if tx_type != 0 {
            out.put_u8(tx_type);
        }
        if tx_type == OPTIMISM_DEPOSITED_TX_TYPE {
            // optimism deposited transactions have no signature
            self.essence.encode(out);
            return;
        }

        // join the essence lists and the signature list into one
        // this allows to reuse as much of the generated RLP code as possible
        rlp_join_lists(&self.essence, &self.signature, out);
    }

    /// Computes the length of the RLP-encoded [Transaction] struct in bytes.
    ///
    /// The computed length includes the lengths of the encoded transaction essence and
    /// signature. If the transaction type (as per EIP-2718) is not zero, an
    /// additional byte is added to the length.
    #[inline]
    fn length(&self) -> usize {
        let tx_type = self.essence.tx_type();
        let payload_length = if tx_type == OPTIMISM_DEPOSITED_TX_TYPE {
            // optimism deposited transactions have no signature
            self.essence.payload_length()
        } else {
            self.essence.payload_length() + self.signature.payload_length()
        };

        let mut length = payload_length + alloy_rlp::length_of_length(payload_length);
        // add the EIP-2718 transaction type for non-legacy transactions
        if tx_type != 0 {
            length += 1;
        }
        length
    }
}

impl<E: TxEssence> Transaction<E> {
    /// Calculates the Keccak hash of the RLP-encoded transaction.
    ///
    /// This hash uniquely identifies the transaction on the Ethereum network.
    #[inline]
    pub fn hash(&self) -> TxHash {
        keccak(alloy_rlp::encode(self)).into()
    }

    /// Recovers the Ethereum address of the sender from the transaction's signature.
    ///
    /// This method uses the ECDSA recovery mechanism to derive the sender's public key
    /// and subsequently their Ethereum address. If the recovery is unsuccessful, an
    /// error is returned.
    #[inline]
    pub fn recover_from(&self) -> anyhow::Result<Address> {
        self.essence.recover_from(&self.signature)
    }
}

/// Joins two RLP-encoded lists into a single RLP-encoded list.
///
/// This function takes two RLP-encoded lists, decodes their headers to ensure they are
/// valid lists, and then combines their payloads into a single RLP-encoded list. The
/// resulting list is written to the provided `out` buffer.
///
/// # Panics
///
/// This function will panic if either `a` or `b` are not valid RLP-encoded lists.
fn rlp_join_lists(a: impl Encodable, b: impl Encodable, out: &mut dyn alloy_rlp::BufMut) {
    let a_buf = alloy_rlp::encode(a);
    let header = alloy_rlp::Header::decode(&mut &a_buf[..]).unwrap();
    if !header.list {
        panic!("`a` not a list");
    }
    let a_head_length = header.length();
    let a_payload_length = a_buf.len() - a_head_length;

    let b_buf = alloy_rlp::encode(b);
    let header = alloy_rlp::Header::decode(&mut &b_buf[..]).unwrap();
    if !header.list {
        panic!("`b` not a list");
    }
    let b_head_length = header.length();
    let b_payload_length = b_buf.len() - b_head_length;

    alloy_rlp::Header {
        list: true,
        payload_length: a_payload_length + b_payload_length,
    }
    .encode(out);
    out.put_slice(&a_buf[a_head_length..]); // skip the header
    out.put_slice(&b_buf[b_head_length..]); // skip the header
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::transactions::EthereumTransaction;

    #[test]
    fn rlp_length() {
        let tx = json!({
          "essence": {
            "Legacy": {
                "nonce": 537760,
                "gas_price": "0x03c49bfa04",
                "gas_limit": "0x019a28",
                "to": { "Call": "0xf0ee707731d1be239f9f482e1b2ea5384c0c426f" },
                "value": "0x06df842eaa9fb800",
                "data": "0x",
                "chain_id": 1
              }
          },
          "signature": {
            "v": 38,
            "r": "0xcadd790a37b78e5613c8cf44dc3002e3d7f06a5325d045963c708efe3f9fdf7a",
            "s": "0x1f63adb9a2d5e020c6aa0ff64695e25d7d9a780ed8471abe716d2dc0bf7d4259"
          }
        });
        let transaction: EthereumTransaction = serde_json::from_value(tx).unwrap();

        let encoded = alloy_rlp::encode(&transaction.essence);
        assert_eq!(encoded.len(), transaction.essence.length());

        let encoded = alloy_rlp::encode(&transaction.signature);
        assert_eq!(encoded.len(), transaction.signature.length());

        let encoded = alloy_rlp::encode(&transaction);
        assert_eq!(encoded.len(), transaction.length());
    }
}
