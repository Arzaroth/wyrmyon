# Negotiation

Detection happens after the key exchange, not before. Rendezvous and PAKE are
the same for every client; only the data transport differs.

1. **Rendezvous**: both peers connect to the public mailbox and meet on the
   nameplate the code starts with.
2. **SPAKE2**: the code becomes a shared key. A wrong code fails here, as with
   any client.
3. **Version exchange**: every client sends an encrypted version message right
   after SPAKE2. wyrmyon adds its capabilities to it:

   ```json
   {"app_versions": {"wyrmyon": {"transports": ["iroh-v1"]}}}
   ```

4. **Decision**: both peers now hold both adverts and decide at the same time,
   so there is no race. Both advertise `iroh-v1`: the iroh-v1 transport
   ([iroh-v1.md](iroh-v1.md)). Otherwise: classic transit, the protocol every
   client speaks.

Legacy clients ignore unknown keys in `app_versions`, so the advert costs them
nothing.

## Downgrade protection

The version messages are encrypted and authenticated with the PAKE key. A
malicious mailbox server cannot strip or alter the advert without knowing the
code, so it cannot push two wyrmyon peers onto classic transit.

## In the code

`Global::config` puts the advert in `app_versions` unless `--force-classic`;
`Global::use_iroh` reads the peer's from `Wormhole::their_app_versions()`.
Each side decides on its own from the same two adverts:

| Our flags | Peer advertises `iroh-v1` | Transport |
| --- | --- | --- |
| none | yes | iroh-v1 |
| none | no | classic transit |
| `--force-classic` | either | classic transit (and no advert, so the peer agrees) |
| `--force-iroh` | no | refused: an `error` to the peer, then a failing exit |

Only a side that saw the peer's advert sends `{"wyrmyon-iroh-v1": ...}`
([iroh-v1.md](iroh-v1.md)), so a legacy client never receives it. When iroh
cannot connect, the transfer fails; it does not fall back to classic transit
on the same wormhole ([decisions.md](../decisions.md)).

## Sources

- [crates/cli/src/lib.rs](../../crates/cli/src/lib.rs) (`config`, `use_iroh`)
- [crates/cli/src/send.rs](../../crates/cli/src/send.rs)
- [crates/cli/src/receive.rs](../../crates/cli/src/receive.rs)
