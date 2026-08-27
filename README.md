# prefixtool

A single-binary CLI for inspecting, splitting and carving up IPv4 and IPv6
prefixes. Built for the moment you are staring at an allocation and need to
know how it divides, what fits inside it, and what is left over afterwards.

Two dependencies: [`ipnet`](https://crates.io/crates/ipnet) for prefix
arithmetic and [`clap`](https://crates.io/crates/clap) for the command line.
Everything else - the allocator, the big-number formatting, the JSON writer -
is in this repo.

```
cargo build --release
./target/release/prefixtool 2001:db8::/52 -56 -64x2
```

## Usage

```
prefixtool [OPTIONS] <PREFIX> [OP]...
```

`PREFIX` is any IPv4 or IPv6 prefix. A bare address is treated as a host route
(`/32` or `/128`), and host bits are cleared with a note rather than rejected.

### Operators

| Operator | Meaning |
| --- | --- |
| `/N` | Split the prefix into `/N` subnets |
| `-N` | Carve one `/N` out of the prefix |
| `-N*K`, `-NxK` | Carve `K` subnets of `/N` |
| `-<prefix>` | Reserve one specific subnet, wherever it sits |
| `+N` | Show the enclosing `/N` supernet |
| `=<addr\|prefix>` | Ask whether an address or prefix falls inside |

Use the `x` form of a count (`-64x2`) in `zsh`, which otherwise tries to glob
the `*`. Flags and operators can be given in any order.

### Options

| Flag | Meaning |
| --- | --- |
| `-n`, `--limit <N>` | Prefixes to list per section (default 8) |
| `-a`, `--all` | List every prefix, however many there are |
| `-q`, `--quiet` | Print prefixes only, one per line, for piping |
| `--json` | Emit a JSON object instead of a report |

### Exit status

| Code | Meaning |
| --- | --- |
| 0 | Success |
| 1 | Bad prefix or operator |
| 3 | A carve request could not be satisfied |

## What it tells you

### Inspecting a prefix

```
$ prefixtool 2001::/64
2001::/64  -  IPv6

  Network        2001::
  Last address   2001::ffff:ffff:ffff:ffff
  Expanded       2001:0000:0000:0000:0000:0000:0000:0000
  Prefix length  /64  (64 host bits)
  Addresses      18,446,744,073,709,551,616 (2^64, ~1.8e19)
  Holds          65,536 x /80   4,294,967,296 x /96   281,474,976,710,656 x /112
  Reverse DNS    0.0.0.0.0.0.0.0.0.0.0.0.1.0.0.2.ip6.arpa
  Ranges         within 2001::/32 - Teredo (RFC 4380)
                 within 2000::/3 - Global unicast (RFC 4291)
  Caution        2001::/64 is Teredo - not for general assignment
```

IPv4 prefixes get the netmask, wildcard mask, broadcast address and usable
host range instead, including the `/31` point-to-point case from RFC 3021.

The `Ranges` lines come from a table of special-purpose registries (RFC 1918,
RFC 6598 CGNAT, RFC 5737 and RFC 3849 documentation space, ULA, link-local,
Teredo, 6to4, NAT64 and friends), and anything you should not be assigning
from raises a `Caution`.

### Splitting

```
$ prefixtool 2001:db8::/52 /64
...
Split 2001:db8::/52 into /64
  Subnets        4,096
  First          2001:db8::/64
  Last           2001:db8:0:fff::/64

    2001:db8::/64
    2001:db8:0:1::/64
    ...
    ... (showing 8 of 4,096; use --all or -n N)
```

Subnets are generated lazily, so `prefixtool ::/0 /128 -q | head` returns
immediately rather than trying to enumerate 2^128 prefixes.

### Carving

```
$ prefixtool 2001:db8::/52 -56 -64x2
...
Carve from 2001:db8::/52
  Request  Assigned             Size
  /56      2001:db8::/56        256 x /64
  /64      2001:db8:0:100::/64  1 x /64
  /64      2001:db8:0:101::/64  1 x /64

  Remaining      70,798,603,754,897,259,102,208 addresses in 10 blocks
  Largest block  2001:db8:0:800::/53  (2,048 x /64)
    2001:db8:0:102::/63
    2001:db8:0:104::/62
    ...
```

All carve operators in one invocation feed a single allocation run:

- Fixed requests (`-10.0.8.0/22`) are placed first, because they have nowhere
  else to go.
- Floating requests (`-24`) are then filled **best-fit** - the smallest free
  block that can still hold the request, lowest address first. That keeps the
  large blocks whole for the large requests.
- The leftovers are aggregated into the fewest possible prefixes.

A `/N` split given alongside a carve describes the *remaining* space, which is
usually the question you actually have:

```
$ prefixtool 10.0.0.0/16 -10.0.8.0/22 -24x4 /24
...
Split the remaining space into /24 (5 free blocks)
  Subnets        248
```

### Locating an address

```
$ prefixtool 2001:db8::/52 /64 =2001:db8:0:3::5
...
Lookup 2001:db8:0:3::5
  yes - inside 2001:db8::/52
  /64 -> 2001:db8:0:3::/64   (subnet #3)
```

### Scripting

`--quiet` prints bare prefixes for piping:

```
$ prefixtool 10.0.0.0/22 /24 -q
10.0.0.0/24
10.0.1.0/24
10.0.2.0/24
10.0.3.0/24
```

`--json` emits everything the report knows, with address counts as exact JSON
numbers (they are far too large for a double, so they are written from exact
decimal digits rather than converted through floating point):

```
$ prefixtool 2001:db8::/52 --json | jq .addresses
75557863725914323419136
```

An unsatisfiable carve exits `3`, so a planning script can just check the
status code:

```
$ prefixtool 10.0.0.0/24 -24 -30 >/dev/null; echo $?
3
```

## Tests

```
cargo test
```

Unit tests cover the allocator, the operator grammar, the special-range table,
the reverse-DNS zones and the big-number formatting; `tests/cli.rs` runs the
real binary and checks its output and exit codes.
