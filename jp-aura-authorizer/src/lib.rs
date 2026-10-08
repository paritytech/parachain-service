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

mod authorization;
mod config;
mod merkle;
mod signing;
mod token;

pub use polkadot_parachain_primitives::primitives::Id as ParaId;

pub use authorization::{authorize, AuthTrace, AuthorizationError, CollatorKey, CollatorSignature};
pub use config::{expected_collator_index, AuthConfig, ConfigError};
pub use merkle::build_collator_tree;
pub use signing::{signable_work_package_hash, WORK_PACKAGE_SIGN_CTX};
pub use token::{AuthToken, SignatureScheme, TokenError};
