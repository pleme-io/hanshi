# hanshi

An orchestrator service for the Roland VersaUV LEF2-300 UV printer, written in Rust with rules in blue.

It accepts print jobs over an HTTP API, writes them to the printer's hot folder, reads printer state over SNMP, and tails the VersaWorks log. Every device interface sits behind a trait in `hanshi-ports`, and the default binary runs against in-memory mocks, so it runs without hardware.

## Crates

| Crate | Role |
|---|---|
| `hanshi-domain` | Job, event and device types |
| `hanshi-ports` | Traits and mocks for the hot folder, SNMP and the log reader |
| `hanshi-core` | Coordinator, stores and the blue rule engine |
| `hanshi-server` | HTTP API |
| `hanshi-bin` | The service binary |

## Build and test

```
cargo test --workspace
cargo run -p hanshi-bin
```

`nix build` builds the same workspace through the flake.

## License

MIT.
