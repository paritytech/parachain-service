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

//! Slot selection and invalid AURA configuration.

use jam_types::Slot;
use jp_aura_authorizer::{expected_collator_index, AuthConfig, ConfigError};

fn config(collator_set_size: u32, slot_duration: u32) -> AuthConfig {
	AuthConfig {
		para_ids: Vec::new(),
		parachain_service: 1337,
		collator_set_root: Default::default(),
		collator_set_size,
		slot_duration,
	}
}

#[test]
fn round_robin_works() {
	let config = config(3, 2);
	for (slot, expected) in [(0, 0), (1, 0), (2, 1), (4, 2), (5, 2), (6, 0)] {
		assert_eq!(expected_collator_index(slot, &config).unwrap(), expected);
	}
	assert_eq!(expected_collator_index(Slot::MAX, &config).unwrap(), (Slot::MAX / 2) % 3);
}

#[test]
fn zero_collator_set_size_errors() {
	for duration in [0, 1] {
		assert!(matches!(
			expected_collator_index(0, &config(0, duration)),
			Err(ConfigError::ZeroCollatorSetSize)
		));
	}
}

#[test]
fn zero_slot_duration_errors() {
	assert!(matches!(
		expected_collator_index(0, &config(1, 0)),
		Err(ConfigError::ZeroSlotDuration)
	));
}
