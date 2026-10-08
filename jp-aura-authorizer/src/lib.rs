// This file is part of Substrate.

// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
// 	http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Shared AURA authorization types, signing payloads, and collator proofs.

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

use codec::{Decode, Encode, MaxEncodedLen};
use jam_types::{Slot, WorkPackage};

mod config;
mod merkle;
mod signing;
mod token;

pub use polkadot_parachain_primitives::primitives::Id as ParaId;

pub use config::{AuthConfig, ConfigError};
pub use merkle::build_collator_tree;
pub use signing::{signable_work_package_hash, WORK_PACKAGE_SIGN_CTX};
pub use token::{AuthToken, CollatorKey, CollatorSignature, SignatureVerifier, TokenError};

/// AURA configuration or token validation failed.
#[derive(Debug)]
pub enum AuthorizationError {
	BadConfig(config::ConfigError),
	BadToken(token::TokenError),
}

/// Select the slot's collator and validate its token.
pub fn authorize<S: token::SignatureVerifier>(
	config: &config::AuthConfig,
	token: &token::AuthToken,
	package: &WorkPackage,
	slot: Slot,
) -> Result<AuthTrace, AuthorizationError> {
	let collator_index =
		config.expected_collator_index(slot).map_err(AuthorizationError::BadConfig)?;
	token
		.try_into_trace::<S>(config, package, collator_index)
		.map_err(AuthorizationError::BadToken)
}

/// What Is-Authorized hands to Refine and Accumulate for every work item in the package.
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode, MaxEncodedLen)]
pub struct AuthTrace {
	pub author_key: token::CollatorKey,
	/// Marks development control packages.
	///
	/// Refine interprets these as commands rather than blocks.
	pub sudo: bool,
}
