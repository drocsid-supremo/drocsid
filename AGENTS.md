# Project Architecture Guide

This repository is a Cargo workspace composed of small, focused crates. Keep the workspace modular by responsibility and dependency ownership.

## Workspace layout

```text
apps/
└── drocsid/              # Final executable and application composition

crates/
├── cli/                  # Command-line parsing and command validation
├── config/               # Shared connection configuration and defaults
├── protocol/             # Wire-format helpers and protocol rules
├── client/               # Client networking and client-side chat state
├── server/               # TCP listener, sessions, broadcast, and history
└── tui/                  # Ratatui interface and terminal input handling
```

The executable package is intentionally kept separate from reusable crates. `apps/drocsid` is responsible for composition, environment loading, command dispatch, and process-level error reporting.

## Dependency boundaries

Keep dependencies directed and acyclic:

```text
protocol
   ↑
server       client
                ↑
               tui

cli ─────────────┐
server ──────────┼──> apps/drocsid
client ──────────┤
tui ─────────────┘
```

Rules:

- `drocsid-protocol` must remain platform-independent and should not depend on UI, networking, or application crates.
- `drocsid-server` may depend on `drocsid-config` and `drocsid-protocol`. It must not depend on `drocsid-client`, `drocsid-tui`, or terminal libraries.
- `drocsid-client` may depend on `drocsid-config` and `drocsid-protocol`. It must not depend on the server or TUI.
- `drocsid-tui` owns Ratatui, Crossterm, and other terminal-specific dependencies. It may consume client state and client networking APIs, but the client must not depend on the TUI.
- `drocsid-cli` owns command parsing and validation. It must not import server, client, or TUI implementation details.
- `drocsid-config` owns shared configuration data and defaults. Environment-file loading belongs to the application layer.
- `apps/drocsid` is the composition root. Cross-component orchestration belongs here rather than in a reusable crate.
- Never introduce circular dependencies. If two crates need the same data, move the stable shared type into `drocsid-protocol` or `drocsid-config` only when that ownership is semantically correct.

## Component design

Treat each crate as a small library with an explicit public API:

- Prefer private implementation modules and expose only types and functions required by consumers.
- Keep transport concerns separate from presentation concerns.
- Keep wire-format rules in `drocsid-protocol` instead of duplicating them in the client and server.
- Keep application composition out of libraries where possible.
- Add a new crate only when it represents a real responsibility, has a stable boundary, or needs an independent dependency set. Do not split files into crates solely to increase the number of components.

## Dependency policy

Before adding a dependency:

1. Confirm that the dependency belongs to the crate that will use it.
2. Check whether the standard library or an existing workspace crate is sufficient.
3. Avoid importing UI, operating-system, or runtime dependencies into protocol and domain-oriented crates.
4. Keep versions consistent with the workspace lockfile and validate all workspace targets.

Examples:

- `ratatui`, `crossterm`, and terminal rendering helpers belong in `drocsid-tui`.
- TCP and session state belong in `drocsid-server` or `drocsid-client`.
- Message encoding, presence events, and mention matching belong in `drocsid-protocol`.
- Runtime addresses and server options are parsed by `drocsid-cli`; process configuration does not depend on `.env` files.

## Code organization

- Use lowercase crate and directory names with hyphens for package names, following Cargo conventions.
- Prefer `lib.rs` as the public entry point for workspace crates.
- Keep tests next to the crate or module they verify.
- Avoid reaching through another crate's private implementation modules.
- When moving a responsibility between crates, update the dependency manifest, imports, README architecture section, and relevant tests in the same change.

## Observability

Observability is part of the runtime contract, not an optional debugging convenience. Network behavior must be diagnosable from structured logs without reproducing the incident locally.

- Use `tracing` for application and library diagnostics; do not add new `println!` or `eprintln!` calls for operational events.
- Initialize the subscriber in `apps/drocsid`, the composition root. Keep human-readable logs as the default and support JSON output through `DROCSID_LOG_FORMAT=json`.
- Respect `RUST_LOG` so operators can increase verbosity for a specific crate or module without recompiling.
- Include stable context in connection-related events: peer address, username when authenticated, error kind, and the processing phase when known.
- Log lifecycle and failure events such as connection acceptance, handshake completion, history delivery, frame reads, broadcasts, and disconnects.
- Do not log message contents, credentials, tokens, or other sensitive payloads by default.
- When reporting a bug, preserve the exact command, relevant `RUST_LOG` value, and the JSON log lines around the failure.

## Test-driven development

Use test-driven development (TDD) for behavior changes, bug fixes, and security fixes. Follow an explicit Red-Green-Refactor cycle:

1. **Red:** express the expected behavior in a focused test at the correct level (unit, integration, or end-to-end), then run it and confirm that it fails for the expected behavioral reason.
2. **Green:** implement the smallest change that makes the test pass, then run the test again and confirm that it passes.
3. **Refactor:** improve the implementation or test structure without changing the specified behavior, while keeping the test suite green.

For behavior that crosses a process, transport, or crate boundary, the Red phase must include a test covering that boundary. A lower-level unit test may supplement it but must not replace it.

Every behavior change and security fix must include a focused test that specifies the expected behavior or reproduces the original failure or security violation. The test must fail against the existing behavior before the implementation and pass after it. Keep regression tests close to the crate or module they protect, and include the relevant test command in the pull request validation.

## Validation

Run the narrowest relevant checks during development. Before merging workspace changes, run:

```bash
cargo fmt --all -- --check
cargo check --all-targets
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets
```

Build the distributable executable explicitly with:

```bash
cargo build --package drocsid --release
```

The release workflow must continue to build the `drocsid` package explicitly so adding libraries to the workspace does not change which binary is published.

Release versioning and crates.io publication are managed by `release-plz`. Pushes to `mosquitao` create or update a release pull request; after that pull request is merged, `release-plz` updates package versions, publishes the workspace crates, and creates a single `vX.Y.Z` tag for the `drocsid` package. The tag dispatches the existing multiplatform binary-release workflow.

Keep the `CARGO_REGISTRY_TOKEN` GitHub Actions secret configured with permission to publish the workspace crates. Do not manually bump versions or publish crates from the release workflow unless the release-plz configuration is intentionally changed.

## GitHub governance

The repository's active GitHub ruleset targets the default branch, `mosquitao`. Changes to this branch must go through a pull request; direct pushes, force pushes, branch deletion, and bypasses are not allowed.

Pull requests targeting `mosquitao` must satisfy all of the following before merging:

- At least one approving review.
- Existing approvals are dismissed when new commits are pushed.
- All review conversations are resolved.
- The `rust` GitHub Actions check passes.
- The branch is up to date with `mosquitao`.
- Linear history is maintained.

Changes should be developed on a separate branch and submitted through a pull request. Do not rely on local validation alone; the required `rust` check is the merge gate configured by the repository ruleset.

The ruleset is managed in the repository settings and is not represented by a tracked repository file. Keep this section synchronized if the GitHub ruleset changes.

## Architecture changes

When adding or changing a component:

1. Define its responsibility and public API.
2. Identify which existing crate owns the data it consumes or produces.
3. Add only the dependencies required by that component.
4. Update the workspace members and dependency graph.
5. Update the architecture documentation.
6. Run the full workspace validation commands.

Prefer small, reversible boundary changes. A broader refactor requires a concrete dependency or ownership problem that the current structure cannot solve cleanly.
