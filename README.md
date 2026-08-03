# drocsid

[![CI](https://img.shields.io/github/actions/workflow/status/drocsid-supremo/drocsid/ci.yml?branch=mosquitao&label=CI)](https://github.com/drocsid-supremo/drocsid/actions/workflows/ci.yml)
[![Rust](https://img.shields.io/badge/built%20with-Rust-000000?logo=rust&logoColor=white)](https://www.rust-lang.org/)
[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Status](https://img.shields.io/badge/status-experimental-orange.svg)](#limitations)

`drocsid` is a small terminal chat application written in Rust. It provides a TCP server and an interactive terminal client built with [Ratatui](https://ratatui.rs/).

The project is experimental and focuses on a simple, self-hosted chat experience with a small protocol and a modular Cargo workspace.

## Features

- Server and client modes in a single executable.
- Real-time message broadcasting over TCP.
- Connected-user presence and join/leave notifications.
- In-memory history with the last 100 messages.
- `@username` mentions with autocomplete.
- Pending-message feedback until the server echoes a message.
- Configurable bind address, server address, and simulated latency.
- Cross-platform release artifacts for desktop platforms and Termux on Android ARM64.

## Requirements

- Rust and Cargo with Rust 2024 edition support.
- A terminal with Unicode and ANSI color support.

Install Rust with [rustup](https://rustup.rs/) if necessary.

## Quick start

Clone the repository and build the executable:

```bash
git clone https://github.com/drocsid-supremo/drocsid.git
cd drocsid
cargo build --package drocsid
```

Start the server in one terminal. It listens on `0.0.0.0:7878` by default:

```bash
cargo run --package drocsid -- server
```

Connect a client from another terminal:

```bash
cargo run --package drocsid -- client --username alice
```

Open more clients with different usernames to test a shared chat session.

The default client address is `127.0.0.1:7878`.

## Installation

After Rust is installed, the executable can be installed from crates.io:

```bash
cargo install drocsid
```

Tagged releases also provide prebuilt archives on the project's [GitHub Releases](https://github.com/drocsid-supremo/drocsid/releases) page.

### Termux

The Android ARM64 archive can be used from Termux:

```bash
pkg install tar
tar -xzf drocsid-vX.Y.Z-android-arm64.tar.gz
chmod +x drocsid
./drocsid client --username alice
```

The published Android artifact targets `aarch64-linux-android`. Other Android architectures are not currently published.

## Command-line interface

The application uses subcommands with conventional long options:

| Command | Option | Default | Description |
| --- | --- | --- | --- |
| `server` | `--listen <ADDR>` | `0.0.0.0:7878` | Local address where the server accepts connections. |
| `server` | `--latency-ms <MS>` | `0` | Artificial delay before broadcasting messages. |
| `client` | `--connect <ADDR>` | `127.0.0.1:7878` | Server address to connect to. |
| `client` | `--username <NAME>` | Required | Username sent during the client handshake. |

Examples:

```bash
drocsid server
drocsid server --listen 0.0.0.0:9000 --latency-ms 25
drocsid client --username alice
drocsid client --connect 3.137.142.191:7878 --username alice
```

Run `drocsid --help` or `drocsid <command> --help` for the complete command reference. Server and client defaults can be overridden per invocation; no `.env` file is required.

For a server running on another machine, pass its reachable address to the client with `--connect` and allow the selected TCP port through the firewall.

## Client controls

| Key | Action |
| --- | --- |
| `Enter` | Send the current message. |
| `Tab` | Apply the selected mention suggestion. |
| `Up` / `Down` | Navigate mention suggestions. |
| `Backspace` | Delete the previous character. |
| `Esc` | Quit the client. |
| `Ctrl+Q` | Force quit the client. |

Type `@` at the beginning of a word to mention another connected user:

```text
@alice are you available?
```

## Architecture

The repository is a Cargo workspace organized by responsibility:

| Component | Responsibility |
| --- | --- |
| `apps/drocsid` | Application composition and executable entry point. |
| `crates/cli` | Startup argument parsing and validation. |
| `crates/config` | Shared connection configuration and defaults. |
| `crates/protocol` | Message, presence, and mention protocol helpers. |
| `crates/client` | TCP client connection and chat state. |
| `crates/server` | TCP listener, sessions, broadcasting, and history. |
| `crates/tui` | Ratatui interface and terminal input handling. |

The client and server communicate through a newline-delimited protocol over a plain TCP stream.

## Development

Format the workspace:

```bash
cargo fmt --all
```

Run the complete local verification suite:

```bash
cargo fmt --all -- --check
cargo check --all-targets
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets
```

The same checks run in [GitHub Actions](https://github.com/drocsid-supremo/drocsid/actions/workflows/ci.yml).

## Releases

Releases are managed by [release-plz](https://release-plz.dev/):

1. Changes are merged into `mosquitao` through pull requests.
2. release-plz creates or updates a release pull request with the required version changes.
3. Merging the release pull request publishes the workspace crates and creates a `vX.Y.Z` tag.
4. The tag starts the multiplatform binary release workflow.

Release automation requires the `CARGO_REGISTRY_TOKEN` GitHub Actions secret.

## Limitations

- The project is experimental and does not promise protocol compatibility yet.
- There is no authentication or authorization.
- Traffic is unencrypted; do not use it for sensitive conversations.
- Message history exists only in memory and is lost when the server stops.
- There is no persistent account, room, or message management.

## License

This project is licensed under the [MIT License](LICENSE).
