// The model's `Hash` images overlap across domains.
//
// Three constructors map into `Hash = { hashBytes: int }` untagged, so
//   x != y  and yet  h(x) == h(y)
//
// This does not break the model: every `Hash`-consuming site is fed by exactly
// one constructor (`accumulate.qnt:357` compares `headHash` to `headHash`), so
// comparisons stay like-for-like. It does block replaying an ITF trace against
// an implementation, which must map a digest back to real bytes and needs the
// domain to know which bytes.
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

  // Injectivity across the domains.
  // Unpatched spec: all three FAIL — h(x) == h(y) for x != y.
  // Domain-tagged spec: all three PASS.
  run injectiveHeadVsCodeTest = assert(hHead != hCode)
  run injectiveHeadVsKeyTest  = assert(hHead != hKey)
  run injectiveCodeVsKeyTest  = assert(hCode != hKey)
}
