# Glossary

**Code**: what the sender reads out and the receiver types,
`7-guitarist-revenge`: a nameplate followed by words. SPAKE2 turns it into the
wormhole key.

**Nameplate**: the number at the start of the code. It names the mailbox both
peers meet on, and is released once they have met.

**Mailbox**: the rendezvous server's channel between the two peers. Carries
only small, encrypted messages; never file data. "The public mailbox" is
`relay.magic-wormhole.io`.

**Wormhole key**: the shared key SPAKE2 derives from the code. Every other key
is derived from it with HKDF.

**Version message**: the first encrypted message after SPAKE2. Its
`app_versions` carries each client's capabilities; wyrmyon advertises
`iroh-v1` there.

**Classic transit**: the data transport every magic-wormhole client speaks:
direct TCP from exchanged hints, or the transit relay, with encrypted records.

**Hint**: an address a peer can be reached at, sent in the `transit` message:
`direct-tcp-v1` (host and port) or `relay-v1` (a transit relay).

**Transit relay**: a server that splices two TCP connections presenting the
same relay token, for peers that cannot reach each other directly.

**Receiver first**: the receiver allocates the code (`receive --new`) and the
sender joins it (`send --code`).

**Record**: one encrypted, length-prefixed message on a transit connection.

**iroh-v1**: wyrmyon's own transport, used only when both peers advertise it.
See [architecture/iroh-v1.md](architecture/iroh-v1.md).

**Node ID**: an iroh endpoint's Ed25519 public key. Fresh for every transfer.

**IrohInfo**: our wire form of an iroh endpoint's address: its node ID, relay
URLs and direct socket addresses, sent as `wyrmyon-iroh-v1` through the mailbox.

**Channel binding**: the MAC exchange on the first iroh stream, keyed from the
wormhole key, that ties the iroh connection to the code.

**Legacy client**: any magic-wormhole client that is not wyrmyon, the Python
`wormhole` CLI first.

**Dilation**: the magic-wormhole protocol extension for a durable, multiplexed
connection. iroh-v1 covers the same ground; legacy-side Dilation is a Later
item.
