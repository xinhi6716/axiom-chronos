# dev.8: cross-platform CI repair

The dev.7 push failed on Ubuntu (25 bridge failures); the default matrix
fail-fast policy cancelled Windows. Earlier Windows local tests and two live
desktop tasks did not establish Linux acceptance.

## Changes

- Explicitly unlock the stable store `flock` before closing its descriptor.
  A concurrent Unix fork can inherit the same open file description; closing
  only the parent's descriptor lets the child retain the lock until exec/exit.
  The process-local mutex, stable inode and cross-process exclusion remain.
  A Unix regression forks while locked and keeps the child alive through a pipe
  while the parent reacquires the released lock, without timing assumptions.
- Await bounded, parallel interrupt acknowledgements before cancellation returns.
  Previously detached sends could lose the race against transport shutdown.
  Acknowledgement is not terminal confirmation; turn owners retain that check,
  and acknowledgement failure marks the server unconfirmed.
- Install ripgrep in both CI environments and run both matrix jobs even if one
  fails. Use the existing Python resolver in the authority integration test.
- Improve fixture diagnostics. The verifier-cancellation fixture stays pending
  after the killed verifier returns, so it actually tests model interruption;
  the separate `attached-late` scenario still tests late completion. No assertion
  accepts a cancelled request as successful or removes terminal checks.
- Bump Rust and plugin manifests together to `0.2.2-codex.dev.8`.

Scope is CI/runtime repair, not a completed product optimization round. Real
five-round cross-version self-iteration remains unaccepted.

## Local validation

Ubuntu 22.04 / Rust 1.99.1: `cargo test --locked` passes all 366 tests,
with 3 existing ignored tests. Includes the actual role authority sequence,
parallel bridge tests, interruption, recovery and the new fork regression.
GitHub's Ubuntu/Windows matrix must also pass before this release is installed.
