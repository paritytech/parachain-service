# D-12: the §5.5 commitment tree pairs adjacent leaves and promotes odd elements
**Spec feedback**: §5.5 must state the pairing rule and the odd-element rule, and should say
explicitly which hash applies to `head_hash` versus to the tree elements. Until then any
independent implementation is likely to compute a different root.
