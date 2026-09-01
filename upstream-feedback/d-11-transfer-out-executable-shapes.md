# D-11: only the deferred transfer mode is executable on the vendored host
**Spec feedback**: §5.1 should say which `transfer_out` shapes are expected to be reachable
in practice. If only the deferred mode ever is, the selectors and `source` are dead wire
fields for the foreseeable future and should be documented as forward-compatibility only.
