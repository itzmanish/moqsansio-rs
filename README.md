# moq

A Rust implementation of [Media over QUIC](https://datatracker.ietf.org/wg/moq/about/) (MoQ) using a **SANS IO** design.

## Design

This project follows the [SANS IO](https://sans-io.readthedocs.io/) pattern: protocol logic is fully separated from network I/O. Each layer produces and consumes byte buffers rather than performing socket operations directly. This makes the code easy to test, embed, and integrate with any async runtime or transport layer.

## Workspace

| Crate | Description |
|---|---|
| [`webtransport`](webtransport/) | WebTransport over HTTP/3 ([draft-ietf-webtrans-http3-14](https://datatracker.ietf.org/doc/draft-ietf-webtrans-http3/)) built on top of [quiche](https://github.com/cloudflare/quiche) |
| [`moq-protocol`](moq-protocol/) | MoQ Transport protocol ([draft-ietf-moq-transport-16](https://datatracker.ietf.org/doc/draft-ietf-moq-transport/)) — control messages, framing, encode/decode with sans-IO `Cursor` + `Vec<u8>` interface |
| [`moq-relay`](moq-relay/) | MoQ relay server — session state machine, subscription management, request routing *(not yet implemented)* |

## Status

**Early development.** The `webtransport` crate has a working implementation of session establishment, stream/datagram management, capsule framing, and flow control. The `moq-protocol` crate implements all control messages from draft-16 (SUBSCRIBE, PUBLISH, FETCH, namespace operations, session control, setup). The relay (`moq-relay`) is next.

## Building

```sh
cargo build --workspace
```

## Testing

```sh
cargo test --workspace
```

## License

See individual crate manifests for details.
