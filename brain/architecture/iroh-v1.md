# The iroh-v1 transport (Planned)

Once both peers agree on `iroh-v1` ([negotiation.md](negotiation.md)), file
data moves over iroh QUIC, authenticated by identities exchanged through the
PAKE channel.

| Aspect | Design |
| --- | --- |
| Endpoint auth | Each side sends its NodeAddr over the encrypted mailbox, pinning the peer's Ed25519 node ID. QUIC/TLS only succeeds against that ID |
| Channel binding | HKDF a confirm key (`wyrmyon/iroh-v1/confirm`) from the wormhole key; both sides exchange MACs on the first stream |
| Connectivity | Hole-punching, relay fallback, connection migration across network changes |
| Multiplexing | A control stream plus one stream per file, both directions, no head-of-line blocking. Covers what Dilation was designed to add |
| Integrity and resume | iroh-blobs: BLAKE3 verified streaming, resuming from the last verified chunk |
| Versioning | ALPN `wyrmyon/1` |

The node ID pin proves the connection reaches the endpoint the peer announced;
the channel binding proves that endpoint belongs to whoever knows the code.
Both are needed: the first alone would trust a NodeAddr a compromised peer
process handed out, the second alone would run over an unauthenticated
connection.

The names `iroh-v1`, `wyrmyon/1` and `wyrmyon/iroh-v1/confirm` are wire format;
changing one breaks older peers.

## Sources

None yet: lands with M4 and M5 in [ROADMAP.md](../../ROADMAP.md).
