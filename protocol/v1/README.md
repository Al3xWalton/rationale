# Proof protocol v1

The Rust host sends one bounded `request.schema.json` evidence slice to the OCaml
kernel. The kernel returns either a proof result or a typed error matching
`response.schema.json`.

## Compatibility

- every message carries `protocol_version: 1`;
- unknown object fields are rejected;
- enum values are closed;
- additive changes require a new schema version unless an existing optional
  field already represents them;
- errors carry no verdict;
- candidate evidence is not part of the kernel protocol and cannot affect proof.

## Canonical representation

Object keys are serialized lexicographically. Array order is semantic:

- nodes sort by record ID;
- edges sort by kind, source ID, target ID, then edge ID;
- proof chains sort by length and then edge IDs;
- gaps sort by gap code;
- conflicts sort by left node ID, right node ID, then rule ID.

The checked fixtures under `fixtures/protocol/` pair readable JSON with the exact
single-line canonical bytes expected from both implementations.
