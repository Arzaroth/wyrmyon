# The wormhole core

`crates/wormhole-core` (package `wyrmyon-wormhole`) speaks the magic-wormhole
client protocol: the mailbox server's WebSocket messages, SPAKE2 on the code,
and the encrypted numbered phases both peers exchange. It is our own
implementation, checked against the Python client (`wormhole` 0.22); see
[decisions.md](../decisions.md) for why it does not reuse magic-wormhole.rs.

## API

| Item | What it does |
| --- | --- |
| `create(&Config, words)` | Binds, allocates a nameplate, claims it, opens the mailbox, sends the PAKE message. Returns a `Pending` holding the generated `Code` |
| `connect(&Config, Code)` | The same with a known code, then `pair()` |
| `Pending::pair()` | Waits for the peer's PAKE message, releases the nameplate, derives the key, exchanges version messages. Returns a `Wormhole` |
| `Pending::abandon()` | Releases the nameplate and closes the mailbox `lonely` |
| `Wormhole::send` / `receive` (and `_json`) | Numbered phases `0, 1, 2...`, encrypted, delivered in order |
| `Wormhole::their_app_versions()` | The peer's `app_versions`, where capability adverts live |
| `Wormhole::key()` / `verifier()` | The wormhole key, for transports to derive their own keys from; the verifier both sides can compare |
| `Wormhole::close(Mood)` | Closes the mailbox with a mood (`happy`, `lonely`, `scary`, `errory`) and waits up to 5 s for `closed` |

`Config` carries the relay URL (`PUBLIC_RELAY` by default), the appid (`APPID`,
`lothar.com/wormhole/text-or-file-xfer`) and our `app_versions`.

## Wire format

Every client frame is JSON with a `type` and a random 2-byte hex `id`; the
server acks each one. The session runs:

1. Server sends `welcome` (`motd`, or `error` to refuse us: `Error::Welcome`).
2. `bind {appid, side, client_version: ["wyrmyon", <version>]}`. The side is
   5 random bytes in hex.
3. `allocate` -> `allocated {nameplate}` (sender only), then
   `claim {nameplate}` -> `claimed {mailbox}`, then `open {mailbox}`.
4. `add {phase: "pake", body}`: body is hex of `{"pake_v1": hex(msg)}`, where
   msg is SPAKE2 symmetric (Ed25519) started with the NFC-normalised code as
   password and the appid as identity.
5. On the peer's `pake`: `release {nameplate}`, finish SPAKE2 into the
   32-byte wormhole key, `add {phase: "version"}` with
   `{"app_versions": ...}` encrypted.
6. The peer's `version` must decrypt, or the code was wrong: close `scary`,
   `Error::WrongCode`.
7. Application messages are phases `"0"`, `"1"`, ... each encrypted with its own
   key.
8. `close {mailbox, mood}` -> `closed`.

The server echoes our own `add`s back as `message`s; the connection drops
anything carrying our side. After pairing, messages from a third side are
ignored.

## Keys

All derivations are HKDF-SHA256 with no salt, 32 bytes, from the wormhole key
(`crypto.rs`):

| Purpose | HKDF info |
| --- | --- |
| Phase key | `wormhole:phase:` + SHA256(side) + SHA256(phase), with the *sender's* side |
| Verifier | `wormhole:verifier` |
| Transit key | `<appid>/transit-key` (used by the transit crates) |

Messages are NaCl secretbox (XSalsa20-Poly1305) with a random 24-byte nonce
prepended. The unit tests pin these derivations to values printed by the
Python client.

## The connection

`connection.rs` owns the WebSocket (boxed, so futures stay small). Reads are
sequential: whoever waits for a server reply (`expect`) buffers any mailbox
`message`s that arrive meanwhile, and `next_message` serves the buffer first.
Server `error` frames become `Error::Server`. There is no reconnection: a dropped
mailbox connection fails the transfer.

## Codes

`code.rs`: `Code` parses `<digits>-<words>` and refuses whitespace; its `Debug`
hides the value. `choose_words` alternates the PGP word list's odd and even
words starting with odd, as the Python client does; `completions` follows the
same rule. `wordlist_data.rs` is generated from the Python client's list.

## Sources

- [crates/wormhole-core/src/lib.rs](../../crates/wormhole-core/src/lib.rs)
- [crates/wormhole-core/src/wormhole.rs](../../crates/wormhole-core/src/wormhole.rs)
- [crates/wormhole-core/src/connection.rs](../../crates/wormhole-core/src/connection.rs)
- [crates/wormhole-core/src/server.rs](../../crates/wormhole-core/src/server.rs)
- [crates/wormhole-core/src/crypto.rs](../../crates/wormhole-core/src/crypto.rs)
- [crates/wormhole-core/src/code.rs](../../crates/wormhole-core/src/code.rs)
