# Issue writing guidelines

Write concise, self-contained issues that explain the bug and expected behavior.

- **Keep titles short.** Name the component and the incorrect behavior. Avoid
  repeating implementation details from the body. Example: “Quint records
  rejected solicitations as successful”.
- **Lead with the problem and consequence.** Explain what happens and why it is
  wrong in one or two sentences.
- **Make issues hermetic.** A reader should understand the issue without access
  to this conversation, a local checkout, saved artifacts, or linked sources.
  Include the relevant behavior directly in the text.
- **Sanitize values.** Use generic actors and small illustrative values that
  preserve the conditions causing the bug. Omit incidental IDs, hashes, exact
  balances, and other details copied from a failing run.
- **Omit reproduction material.** Do not include seeds, trace or frame numbers,
  local paths, revision pins, replay commands, stack traces, or investigation
  and test-run history. Describe the causal sequence instead.
- **Use structure only where it helps.** A short paragraph and a few bullets
  may suffice. For an ordering-dependent bug, use a numbered sequence showing
  the precondition, action, actual outcome, and failure.
- **Distinguish attempts from successful effects.** State which layer accepts,
  rejects, or records an operation. Be precise about whether Quint, Rust, JAM,
  or the replay is responsible and whether their actual states differ.
- **End with expected behavior.** State the correction or invariant that must
  hold. Include implementation guidance only when it clarifies the fix; avoid
  unnecessary prescriptions.

## Suggested shape

```text
<Component> <incorrect behavior>

<Problem and consequence in one or two sentences.>

The failing sequence is:

1. <Generic precondition, with simple example values if useful.>
2. <Operation and actual behavior.>
3. <Incorrect bookkeeping or effect and its consequence.>

Expected: <Required behavior and any essential constraint.>
```

Use this shape when a sequence helps explain the bug; shorter issues do not
need to fill every part.
