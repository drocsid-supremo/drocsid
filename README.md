# drocsid

[![Rust](https://img.shields.io/badge/built%20with-Rust%202024-000000?logo=rust&logoColor=white)](https://www.rust-lang.org/)
[![TUI](https://img.shields.io/badge/interface-terminal%20TUI-0f766e)](https://ratatui.rs/)
[![CI](https://img.shields.io/badge/CI-GitHub%20Actions-2088ff?logo=githubactions&logoColor=white)](.github/workflows/ci.yml)
[![Status](https://img.shields.io/badge/status-experimental-f59e0b)](#limitations)

A small terminal chat application written in Rust. drocsid uses a TCP server and a lightweight terminal user interface (TUI) client built with [Ratatui](https://ratatui.rs/).

## Features

- Dedicated TCP server and terminal client modes.
- Multiple clients connected to the same chat session.
- Real-time message broadcasting.
- Connected-user list in the session sidebar.
- Join and leave notifications.
- In-memory message history, with the last 100 messages retained by the server.
- User mentions with `@username` highlighting.
- Mention autocomplete with `Tab` and selection with the arrow keys.
- Pending-message styling until the server echoes a sent message.
- Configurable server bind address, client connection address, and simulated latency.
- Graceful disconnect feedback in the client interface.

## Requirements

- Rust and Cargo with support for the Rust 2024 edition.
- A terminal capable of displaying Unicode and ANSI colors.

Install Rust through [rustup](https://rustup.rs/) if it is not already available.

## Getting started

Clone the repository and enter the project directory:

```bash
git clone <repository-url>
cd discord
```

Build the application:

```bash
cargo build
```

### Start the server

In one terminal, run:

```bash
cargo run -- mode=server
```

The server listens on `0.0.0.0:7878` by default.

### Connect a client

In another terminal, run:

```bash
cargo run -- mode=client username=alice
```

Open additional terminals and connect more users by changing the username:

```bash
cargo run -- mode=client username=bob
```

The client connects to `127.0.0.1:7878` by default.

### Termux / Android

Each tagged GitHub Release includes an Android ARM64 archive for Termux:

```text
drocsid-vX.Y.Z-android-arm64.tar.gz
```

On an ARM64 Android device with Termux, download the archive from the project's GitHub Release and run:

```bash
pkg install tar
tar -xzf drocsid-vX.Y.Z-android-arm64.tar.gz
chmod +x drocsid
./drocsid mode=client username=alice
```

The Android artifact targets `aarch64-linux-android`, so it is built against the Android runtime rather than the standard Linux runtime. Termux ARM64 is supported by the release artifact; other Android architectures are not currently published.

## Command-line arguments

The application uses `key=value` arguments:

| Argument | Required | Description |
| --- | --- | --- |
| `mode=server` | Yes | Starts the TCP chat server. |
| `mode=client` | Yes | Starts the terminal chat client. |
| `username=<name>` | Client only | Sets the username sent during the client handshake. |

Examples:

```bash
# Server
cargo run -- mode=server

# Client
cargo run -- mode=client username=alice
```

An unknown mode or a client without a non-empty username causes the application to exit with an error.

## Configuration

Configuration is loaded from environment variables. You can copy the example file and adjust it for your environment:

```bash
cp .env.example .env
```

| Variable | Default | Description |
| --- | --- | --- |
| `SERVER_BIND_ADDR` | `0.0.0.0:7878` | Local address and port used by the server to accept connections. |
| `SERVER_CONNECT_ADDR` | `127.0.0.1:7878` | Address and port used by clients to connect to the server. |
| `SERVER_SIMULATED_LATENCY_MS` | `0` | Artificial delay, in milliseconds, applied by the server before broadcasting received messages. Invalid values fall back to `0`. |

For a server running on another machine, set `SERVER_CONNECT_ADDR` to that machine's reachable address in the client's `.env` file. Make sure the selected TCP port is allowed by the host firewall.

## Client controls

| Key | Action |
| --- | --- |
| `Enter` | Send the current message. |
| `Tab` | Apply the selected mention suggestion. |
| `Up` / `Down` | Move through mention suggestions. |
| `Backspace` | Remove the previous character. |
| `Esc` | Quit the client. |
| `Ctrl+Q` | Force quit the client. |
| `Enter` after a disconnect | Close the disconnect notice. |

To mention another connected user, type `@` at the beginning of a word. For example:

```text
@alice are you available?
```

The client highlights messages that mention the current user's exact username. Partial matches such as `@alice1` do not trigger a highlight for `alice`.

## Development

Format the code:

```bash
cargo fmt --all
```

Run the full local verification suite:

```bash
cargo fmt --all -- --check
cargo check --all-targets
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets
```

These checks are also executed by the GitHub Actions workflow in `.github/workflows/ci.yml`.

## Architecture

The application is organized into three main areas:

- `src/server`: accepts TCP connections, tracks clients, broadcasts messages, and keeps in-memory history.
- `src/client`: manages the TCP connection and renders the interactive Ratatui interface.
- `src/protocol.rs`: contains the text protocol helpers for chat messages, user-presence events, and mentions.

The server and client communicate over a plain TCP stream. Chat messages and presence events are newline-delimited.

## Limitations

This project is currently experimental. In particular:

- There is no authentication or authorization.
- Traffic is not encrypted; do not use it for sensitive conversations.
- Message history is held only in memory and is lost when the server stops.
- There is no persistent account or room management.
- The protocol is intentionally small and does not provide an API compatibility guarantee.

## License

This project is licensed under the [MIT License](LICENSE).
