# Servers (Planned)

The mailbox is always the public one. Data relays are negotiated in-band, so
they need no discovery.

| Server | Used for | How it is chosen |
| --- | --- | --- |
| `relay.magic-wormhole.io` | Rendezvous (mailbox) | Fixed. The sender cannot know which client the receiver runs, so codes must live where everyone looks |
| `transit.magic-wormhole.io` | Classic data relay | Legacy path only |
| iroh relays (n0's public ones at first) | iroh data relay | The relay URL travels inside the encrypted NodeAddr; moving to self-hosted relays is a client default change |
| Our own mailbox | Fallback rendezvous (Later) | Only when the public mailbox is unreachable. Its codes carry a prefix (`m7-guitarist-revenge`) that legacy clients reject as malformed, which is correct |

## Sharing the public mailbox

The mailbox is a community service; the rules are in [CLAUDE.md](../../CLAUDE.md):

- The official appid `lothar.com/wormhole/text-or-file-xfer`, so nameplates are
  visible to legacy clients.
- Nothing nonstandard until the peer's version message advertises `iroh-v1`.
- Small traffic: file data never goes through the mailbox.
- The server's welcome message (MOTD, errors) reaches the user.

## Sources

None yet: the mailbox client lands with M1 in [ROADMAP.md](../../ROADMAP.md).
