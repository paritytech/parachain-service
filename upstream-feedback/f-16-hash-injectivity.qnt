// Disproof: the model's `Hash` is not a hash.
//
// Defining property of a hash: distinct preimages give distinct digests.
//   x != y  =>  h(x) != h(y)
//
// `Hash` is `{ hashBytes: int }`, and three constructors map into it untagged,
// so the property fails by construction.
//
//   quint test hash_injectivity.qnt
//   -> the three `witness` runs pass, the three `injective` runs fail.
module hash_injectivity {
  import types.* from "./types"

  // Three distinct preimages, from three distinct domains.
  pure val headOne: HeadData           = 1                // a parachain head
  pure val codeOne: ValidationCodeHash = { vchBytes: 1 }  // a validation code
  pure val keyOne:  List[int]          = List(1)          // a key_value_storage key

  // Their digests, under the model's own constructors.
  pure val hHead: Hash = headHash(headOne)   // types.qnt:191
  pure val hCode: Hash = vchAsHash(codeOne)  // types.qnt:59
  pure val hKey:  Hash = listHash(keyOne)    // types.qnt:195

  // All three digests are the same value. These PASS.
  run witnessHeadTest = assert(hHead == { hashBytes: 1 })
  run witnessCodeTest = assert(hCode == { hashBytes: 1 })
  run witnessKeyTest  = assert(hKey  == { hashBytes: 1 })

  // Injectivity across the domains. These FAIL: h(x) == h(y) for x != y.
  run injectiveHeadVsCodeTest = assert(hHead != hCode)
  run injectiveHeadVsKeyTest  = assert(hHead != hKey)
  run injectiveCodeVsKeyTest  = assert(hCode != hKey)
}
