# F-14: no error code exists for a sender-side transfer-gas cap
Either `TransferError` needs a variant for a refused gas request, or §5.1 must state that the
service may not impose such a cap and instead relies on a per-digest cumulative budget
(F-13). The two findings should be resolved together.
