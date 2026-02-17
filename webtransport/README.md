# webtransport

A SANS IO implementation of [WebTransport over HTTP/3](https://datatracker.ietf.org/doc/draft-ietf-webtrans-http3/) (draft-ietf-webtrans-http3-14) designed to layer on top of [quiche](https://github.com/cloudflare/quiche).

## Design

This crate implements the WebTransport protocol logic without performing any network I/O. It produces and consumes byte buffers that the caller feeds to/from a `quiche::Connection`. This SANS IO approach means:

- **No async runtime dependency** - works with tokio, async-std, mio, or blocking I/O
- **Fully testable** - all protocol logic can be exercised with in-memory buffers
- **Embeddable** - integrate into any application architecture

## Architecture

```
                    +-----------------+
  Application  --> | Connection      |  WebTransport session & stream management
                    +-----------------+
                    | H3Connection    |  HTTP/3 framing, SETTINGS, CONNECT
                    +-----------------+
                    | quiche          |  QUIC transport (external)
                    +-----------------+
```

### Modules

| Module | Description |
|---|---|
| `connection` | Top-level `Connection` state machine. Manages sessions, streams, datagrams, and emits `Event`s to the caller |
| `session` | Per-session state: lifecycle (`Connecting` / `Established` / `Draining` / `Closed`), stream tracking, capsule parsing |
| `stream` | WebTransport stream header encoding/decoding (signal value + session ID) with incremental byte-by-byte parsing |
| `capsule` | Capsule encode/decode: `CLOSE_SESSION`, `DRAIN_SESSION`, `WT_MAX_STREAMS`, `WT_MAX_DATA`, `WT_STREAMS_BLOCKED`, `WT_DATA_BLOCKED` |
| `flow_control` | Session-level flow control: stream limits (bidi/uni) and data limits with monotonicity enforcement |
| `frame` | Wire constants from draft-ietf-webtrans-http3-14: SETTINGS IDs, frame types, capsule types, error codes |
| `config` | `Config` struct for tuning session limits, buffering, and datagram support |
| `error` | Error types and WebTransport-to-HTTP/3 application error code mapping (section 4.4) |
| `varint` | QUIC variable-length integer encoding/decoding (RFC 9000 section 16) with incremental `VarintDecoder` |
| `h3` | Minimal HTTP/3 layer: frame parsing, SETTINGS exchange, QPACK encode/decode (static table + Huffman), CONNECT request/response |

## Usage

```rust
use webtransport::{Config, Connection, Event};

// Create a server-side WebTransport connection
let config = Config::default();
let mut wt = Connection::new(true, config);

// Initialize H3 — returns (stream_id, data) pairs to write to quiche
let init_streams = wt.initialize().unwrap();
for (stream_id, data) in init_streams {
    // quiche_conn.stream_send(stream_id, &data, false).unwrap();
}

// Feed incoming QUIC stream data into the WebTransport layer
// wt.process_stream_data(stream_id, &data, fin).unwrap();

// Poll for events
while let Some(event) = wt.poll_event() {
    match event {
        Event::SessionRequest { session_id, authority, path, .. } => {
            // Accept or reject the session
            let response_data = wt.accept_session(session_id, &[]).unwrap();
            // quiche_conn.stream_send(session_id, &response_data, false).unwrap();
        }
        Event::UniStreamReceived { session_id, stream_id } => {
            // Handle incoming unidirectional stream
        }
        Event::BidiStreamReceived { session_id, stream_id } => {
            // Handle incoming bidirectional stream
        }
        Event::Datagram { session_id, payload } => {
            // Handle incoming datagram
        }
        _ => {}
    }
}
```

## Spec Coverage

Implements [draft-ietf-webtrans-http3-14](https://datatracker.ietf.org/doc/draft-ietf-webtrans-http3/). The reference text is included as [`draft-webtrans-h3-14.txt`](draft-webtrans-h3-14.txt).

| Section | Feature | Status |
|---|---|---|
| 3.1 | SETTINGS negotiation (`WT_MAX_SESSIONS`, `H3_DATAGRAM`, `ENABLE_CONNECT_PROTOCOL`) | Implemented |
| 3.2 | Session establishment via extended CONNECT | Implemented |
| 3.3 | Application protocol negotiation (`WT-Available-Protocols` / `WT-Protocol`) | Implemented |
| 3.4 | PRIORITY_UPDATE framing | Implemented |
| 4.2 | Unidirectional streams (type `0x54`) | Implemented |
| 4.3 | Bidirectional streams (signal `0x41`) | Implemented |
| 4.4 | Error code mapping (WebTransport <-> HTTP/3) with RESET_STREAM_AT | Implemented |
| 4.5 | Datagrams via Quarter Stream ID | Implemented |
| 4.6 | Buffering incoming streams/datagrams before session established | Implemented |
| 4.7 | GOAWAY handling and `WT_DRAIN_SESSION` capsule | Implemented |
| 4.8 | TLS exporter context construction | Implemented |
| 5 | Session-level flow control (stream limits + data limits) | Implemented |
| 5.6 | Flow control capsules (`WT_MAX_STREAMS`, `WT_MAX_DATA`, `WT_STREAMS_BLOCKED`, `WT_DATA_BLOCKED`) | Implemented |
| 6 | Session termination via `CLOSE_SESSION` capsule | Implemented |
| 7.1 | 0-RTT settings validation (limits must not decrease) | Implemented |

## Dependencies

- [`quiche`](https://crates.io/crates/quiche) `0.22` - QUIC transport
- [`thiserror`](https://crates.io/crates/thiserror) `2.0` - Error derive macros
