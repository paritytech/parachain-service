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

//! SCALE-encoded authorization output shared by verifier and service.

use codec::{Decode, Encode, MaxEncodedLen};

/// Raw public key bytes for either supported signature scheme.
pub type CollatorKey = [u8; 32];

/// A collator's signature over an authorization token's signing payload.
///
/// Encoded as 64 raw bytes.
pub type CollatorSignature = [u8; 64];

/// What Is-Authorized hands to Refine and Accumulate for every work item in the package.
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode, MaxEncodedLen)]
pub struct AuthTrace {
	pub author_key: CollatorKey,
	/// Marks development control packages.
	///
	/// Refine interprets these as commands rather than blocks.
	pub sudo: bool,
}
