# Negotiation (Planned)

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

`--force-classic` and `--force-iroh` override the decision for testing and
debugging only.

## Sources

None yet: lands with M4 in [ROADMAP.md](../../ROADMAP.md).
