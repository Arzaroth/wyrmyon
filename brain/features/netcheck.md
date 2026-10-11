# Checking the network

```
wyrm netcheck                          # against n0's relays
wyrm --iroh-relays disabled netcheck   # what a relay-less transfer can use
```

`wyrm netcheck` says how this machine's network lets two wyrms connect over
iroh, in the spirit of `tailscale netcheck`, so a transfer that ends up
relayed can be explained before it starts:

```
Local addresses 192.168.1.5:51109, [2001:db8::137]:39702
UDP             IPv4 yes, IPv6 yes
Public address  203.0.113.8:51109, [2001:db8::137]:39702
NAT mapping     the same for every server (an easy NAT)
Port mapping    UPnP yes, PCP yes, NAT-PMP yes
Relays          https://euc1-1.relay.n0.iroh.link./ 23 ms
                https://use1-1.relay.n0.iroh.link./ 90 ms

Direct connections should open with most peers (public IPv6).
```

| Line | Where it comes from |
| --- | --- |
| Local addresses | The addresses a fresh endpoint would announce in `IrohInfo` |
| UDP, Public address, NAT mapping, Captive portal | iroh's net report: QUIC address discovery (QAD) against the relays, which reports the address and port each relay sees; a mapping that differs between relays is a hard NAT |
| Port mapping | A probe of the gateway for UPnP, PCP and NAT-PMP (the `portmapper` crate iroh uses itself) |
| Relays | The latency of each relay the report measured; iroh stops once its three fastest answer, so others say `not measured` |

The last line is a verdict: no relay answering, no UDP answer, a public IPv6
address, an easy NAT, a hard NAT with or without port mapping. A hard NAT
still connects directly to a peer on an easy NAT or a public address; two
hard NATs stay on the relay, since iroh does not spray ports the way
Tailscale's birthday-paradox probing does. With `--iroh-relays disabled` only
the local addresses and port mapping are known, and the verdict says what a
relay-less transfer needs: one side able to reach the other
([architecture/iroh-v1.md](../architecture/iroh-v1.md), both peers dial).

A relay without QUIC address discovery, such as a plain-HTTP relay of your
own, answers latency probes but no UDP ones, so the UDP line says no and the
verdict says why that may be.

The net report is behind iroh's `unstable-net-report` feature, outside its
semver promise: an iroh upgrade can break the build here, never the wire.

## Sources

- [crates/transport-iroh/src/netcheck.rs](../../crates/transport-iroh/src/netcheck.rs)
- [crates/cli/src/netcheck.rs](../../crates/cli/src/netcheck.rs)
