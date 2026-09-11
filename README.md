# prefixtool and fabrictool

[![CI](https://github.com/jiphex/network-prefix-cli/actions/workflows/ci.yml/badge.svg)](https://github.com/jiphex/network-prefix-cli/actions/workflows/ci.yml)

Two CLIs for the two things you end up doing on a whiteboard before anything
gets built.

**`prefixtool`** inspects, splits and carves up IPv4 and IPv6 prefixes. Built
for the moment you are staring at an allocation and need to know how it
divides, what fits inside it, and what is left over afterwards.

**`fabrictool`** sizes the cabling between switches. Built for the moment you
are staring at a rack diagram and need to know how many cables, transceivers
and ports a mesh of eight of them actually costs, whether breaking a 400G port
into four 100G lanes makes that better or worse, and what the whole thing is
oversubscribed by once the servers are plugged in.

They ship together, in one archive, and read the same way: a thing to work on,
then operators for the questions you want answered about it.

## Install

Grab the archive for your platform from the
[releases page](https://github.com/jiphex/network-prefix-cli/releases), unpack
it and put `prefixtool` and `fabrictool` on your `PATH`. Each archive ships
with a `.sha256` next to it. Builds are published for Linux (x86-64 gnu and static musl,
arm64), macOS (Intel and Apple silicon) and Windows.

### Homebrew

```
brew tap jiphex/network-prefix-cli https://github.com/jiphex/network-prefix-cli
brew install jiphex/network-prefix-cli/prefixtool
```

The formula lives in this repository under `Formula/`, so the tap needs the
repository URL spelled out - Homebrew otherwise goes looking for a repository
called `homebrew-network-prefix-cli`. It is named after `prefixtool` and
installs both binaries, prebuilt for your platform, so there is no Rust
toolchain and no compile.

Homebrew clears the quarantine flag itself, so the macOS note below does not
apply to a `brew install`.

### Nix

The repository is a flake, so it can be run without installing anything:

```
nix run github:jiphex/network-prefix-cli -- 2001:db8::/52 -56 -64x2
nix run github:jiphex/network-prefix-cli#fabrictool -- 8 @100G %400G
```

Or built, or brought into a profile or a NixOS configuration:

```
nix build github:jiphex/network-prefix-cli
nix profile install github:jiphex/network-prefix-cli
```

`nix develop` gives a shell with cargo, rustc, clippy, rustfmt and
rust-analyzer, and `nix flake check` builds the package - which runs the test
suite, since that is part of `buildRustPackage`.

The flake takes its name, version, description and homepage straight from
`Cargo.toml`, so there is no second copy of the version to keep in step. Its
only input is nixpkgs, tracking `nixos-unstable` because the crate is on the
2024 edition.

### macOS

The macOS binaries are ad-hoc signed but not notarized, so Gatekeeper will
object to one carrying a quarantine flag. Unpacking from Terminal never sets
that flag:

```
tar xzf prefixtool-<tag>-aarch64-apple-darwin.tar.gz
./prefixtool --version
./fabrictool --version
```

If you downloaded through a browser and hit *"Apple could not verify..."*, the
macOS archives bundle a script for it:

```
./macos-unquarantine.sh
```

It does both binaries: clears the quarantine flag, repairs the ad-hoc signature
if it needs it, and runs each one to prove the result works. The equivalent by
hand is `xattr -d com.apple.quarantine prefixtool fabrictool`.

Or build it yourself:

```
cargo install --git https://github.com/jiphex/network-prefix-cli
```

From a checkout:

```
cargo build --release
./target/release/prefixtool 2001:db8::/52 -56 -64x2
./target/release/fabrictool 8 @100G %400G
```

## prefixtool

```
prefixtool [OPTIONS] <PREFIX> [OP]...
```

`PREFIX` is any IPv4 or IPv6 prefix. A bare address is treated as a host route
(`/32` or `/128`), and host bits are cleared with a note rather than rejected.

### Operators

| Operator | Meaning |
| --- | --- |
| `/N` | Split the prefix into `/N` subnets |
| `%M` | Split it into `M` subnets, whatever lengths that needs |
| `%a:b:c` | Share it out in that ratio |
| `-N` | Carve one `/N` out of the prefix |
| `-N*K`, `-NxK` | Carve `K` subnets of `/N` |
| `-<prefix>` | Reserve one specific subnet, wherever it sits |
| `-N:name`, `-<prefix>:name` | Name a carve, so the map reads as a plan |
| `+N` | Show the enclosing `/N` supernet |
| `+<prefix>` | Aggregate; several `+` make one aggregate covering them all |
| `=<addr\|prefix>` | Ask whether an address or prefix falls inside |
| `@N` | The Nth subnet of a requested split; `@-1` is the last |
| `^N` | The prefix `N` blocks along at the same size; `^-1` is the previous |
| `.` | The reverse DNS zones covering it; `.N` picks the boundary |

Use the `x` form of a count (`-64x2`) in `zsh`, which otherwise tries to glob
the `*`. Flags and operators can be given in any order.

### Options

| Flag | Meaning |
| --- | --- |
| `-n`, `--limit <N>` | Prefixes to list per section (default 8) |
| `-a`, `--all` | List every prefix, however many there are |
| `--from <end>` | Which end floating carves fill from: `bottom` (default) or `top` |
| `-q`, `--quiet` | Print prefixes only, one per line, for piping |
| `--json` | Emit a JSON object instead of a report |
| `--color <when>` | `auto` (default), `always` or `never` |

### Exit status

| Code | Meaning |
| --- | --- |
| 0 | Success |
| 1 | Bad prefix or operator |
| 3 | A carve request could not be satisfied |
| 4 | `--quiet`, and an `=<addr>` asked about is outside the prefix |

Under `--quiet` an `=<addr>` is a question, so its answer becomes the exit
status and the tool can stand in for a test:

```
if prefixtool 10.0.0.0/8 =$addr -q > /dev/null; then
    echo "$addr is ours"
fi
```

Outside is **4** rather than 1 so that it stays distinct from bad input: a
mistyped address is a different thing from a confident no, and a script
checking for one should never be handed the other. With several `=` operators,
any one outside is a fail. The other output modes print the answer for you to
read, so they stay at 0.

## What prefixtool tells you

### Inspecting a prefix

```
$ prefixtool 2001::/64
2001::/64  -  IPv6

  Network        2001::
  Last address   2001::ffff:ffff:ffff:ffff
  Expanded       2001:0000:0000:0000:0000:0000:0000:0000
  Prefix length  /64  (64 host bits)
  Addresses      2^64 (~1.8e19)
  Holds          65,536 x /80 or 4,294,967,296 x /96 or 2^48 x /112
  Reverse DNS    0.0.0.0.0.0.0.0.0.0.0.0.1.0.0.2.ip6.arpa.
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

### Splitting into a count

`/N` asks for subnets of a given size. `%M` asks for a given number of them and
works out the sizes, which is the question you have when the space is being
shared between a fixed number of parties:

```
$ prefixtool 10.0.0.0/24 %5
...
Split 10.0.0.0/24 into 5
  Sizes          3 x /26 and 2 x /27
  Note           as even as the space allows - an exact split needs a power of two
  First          10.0.0.0/27
  Last           10.0.0.192/26

    10.0.0.0/27
    10.0.0.32/27
    10.0.0.64/26
    10.0.0.128/26
    10.0.0.192/26
```

The pieces always tile the prefix exactly, and never use more than two lengths,
one bit apart. When `M` is a power of two the result is the uniform split `/N`
would have given. Like `/N`, a `%M` alongside a carve divides what the carve
left over.

### Splitting into a ratio

`%M` shares the space equally. `%a:b:c` shares it out in proportion, which is
the question you have when the parties are not equal - a regional site that
needs twice what the branches do:

```
$ prefixtool 2001:db8::/48 %2:1:1
...
Share 2001:db8::/48 in the ratio 2:1:1
  Ratio          2:1:1  as asked

  Share 1  2 of 4 parts, 2^79 (~6.0e23) addresses, 1 block
    2001:db8::/49

  Share 2  1 of 4 parts, 2^78 (~3.0e23) addresses, 1 block
    2001:db8:0:8000::/50

  Share 3  1 of 4 parts, 2^78 (~3.0e23) addresses, 1 block
    2001:db8:0:c000::/50
```

A share gets as many blocks as its portion needs; the shares tile the space
exactly between them, and they come out in the order they were written.

Under `--quiet` a ratio prints one line per share, blocks separated by spaces,
because a share is the unit you asked for and a flat list would lose where
each one ends:

```
$ prefixtool 10.0.0.0/24 %3:1 -q
10.0.0.0/25 10.0.0.128/26
10.0.0.192/26
```

So `while read -a blocks` gets one share per iteration. Those lines are never
cut short by `-n`: half a share's blocks is a wrong answer rather than a short
one, and the ratio already says how many lines to expect.

A ratio is exactly cuttable when its parts, reduced by their common factor, add
up to a power of two. `2:1:1` is, and so is `3:1` and `6:2`. `2:1` is not -
two thirds of a prefix is not a prefix - so it lands on the nearest aligned
split and the report says which one that is rather than pretending:

```
$ prefixtool 10.0.0.0/24 %2:1
...
Share 10.0.0.0/24 in the ratio 2:1
  Ratio          3:1  for a request of 2:1
  Note           the nearest aligned split - an exact one needs shares that
                 add up to a power of two once reduced

  Share 1  2 of 3 parts, 192 addresses, 2 blocks
    10.0.0.0/25
    10.0.0.128/26

  Share 2  1 of 3 parts, 64 addresses, 1 block
    10.0.0.192/26
```

That is the same bargain `%M` already makes - `%3` hands three equal parties a
half and two quarters - and `%1:1:1` and `%3` do in fact produce the same
blocks. Like `%M`, a ratio alongside a carve shares out what the carve left.

### Carving

```
$ prefixtool 2001:db8::/52 -56 -64x2
...
Carve from 2001:db8::/52
  Request  Assigned             Size
  /56      2001:db8::/56        256 x /64
  /64      2001:db8:0:100::/64  1 x /64
  /64      2001:db8:0:101::/64  1 x /64

  Remaining      ~7.1e22 addresses in 10 blocks
  Largest block  2001:db8:0:800::/53  (2,048 x /64)

Map of 2001:db8::/52
  -> 2001:db8::/56         carved
  -> 2001:db8:0:100::/64   carved
  -> 2001:db8:0:101::/64   carved
     2001:db8:0:102::/63
     2001:db8:0:104::/62
     2001:db8:0:108::/61
     ... 7 blocks, ~7.1e22 addresses (use --all)
```

The map underneath shows the parent laid out block by block, with the
allocations marked in place, so you can see where a carve landed rather than
cross-referencing two lists by address:

```
$ prefixtool 2001:db8::/56 -2001:db8:0:cc::/64
...
Map of 2001:db8::/56
     2001:db8::/57
     2001:db8:0:80::/58
     2001:db8:0:c0::/61
     2001:db8:0:c8::/62
  -> 2001:db8:0:cc::/64   carved
     2001:db8:0:cd::/64
     2001:db8:0:ce::/63
     2001:db8:0:d0::/60
     2001:db8:0:e0::/59
```

The allocations and the free blocks tile the parent exactly, so every address
is accounted for on exactly one line. Long runs away from an allocation are
elided into a line that still counts what it hid; `--all` shows everything.

All carve operators in one invocation feed a single allocation run:

- Fixed requests (`-10.0.8.0/22`) are placed first, because they have nowhere
  else to go.
- Floating requests (`-24`) are then filled **best-fit** - the smallest free
  block that can still hold the request, lowest address first. That keeps the
  large blocks whole for the large requests.
- The leftovers are aggregated into the fewest possible prefixes.

Any carve can be given a name, which turns the map from a picture into a plan
somebody else can read:

```
$ prefixtool 10.0.0.0/16 -24:dmz -22:wifi -10.0.8.0/22:legacy
...
Carve from 10.0.0.0/16
  Request      Assigned      Name    Size
  /24          10.0.12.0/24  dmz     256 addresses, 254 usable
  /22          10.0.0.0/22   wifi    1,024 addresses, 1,022 usable
  10.0.8.0/22  10.0.8.0/22   legacy  1,024 addresses, 1,022 usable

  Remaining      63,232 addresses in 7 blocks
  Largest block  10.0.128.0/17  (32,768 addresses, 32,766 usable)

Map of 10.0.0.0/16
  -> 10.0.0.0/22    wifi
     10.0.4.0/22
  -> 10.0.8.0/22    legacy
  -> 10.0.12.0/24   dmz
     10.0.13.0/24
     10.0.14.0/23
     10.0.16.0/20
     ... 3 blocks, 57,344 addresses (use --all)
```

The name column only appears when something was named. Names are letters,
digits, `-`, `_` and `.`, so they need no quoting. On an IPv6 prefix the whole
payload is read as an address first, so `-2001:db8::1` stays an address rather
than becoming `2001:db8:` named `1`; write the length out (`-2001:db8::1/128:lo`)
to name a host route.

`--from=top` fills floating carves from the far end of the prefix instead.
Infrastructure is usually taken down from the top so that it grows towards the
customer allocations coming up from the bottom rather than into them:

```
$ prefixtool 10.0.0.0/16 -22:infra --from=top
...
Carve from 10.0.0.0/16, filling from the top
  Request  Assigned       Name   Size
  /22      10.0.252.0/22  infra  1,024 addresses, 1,022 usable

  Remaining      64,512 addresses in 6 blocks
  Largest block  10.0.0.0/17  (32,768 addresses, 32,766 usable)

Map of 10.0.0.0/16
     ... 3 blocks, 57,344 addresses (use --all)
     10.0.224.0/20
     10.0.240.0/21
     10.0.248.0/22
  -> 10.0.252.0/22   infra
```

It steers floating requests only: a fixed `-<prefix>` has nowhere else to go
either way. Everything else is unchanged, and the two directions produce
mirror images of each other.

A `/N` split given alongside a carve describes the *remaining* space, which is
usually the question you actually have:

```
$ prefixtool 10.0.0.0/16 -10.0.8.0/22 -24x4 /24
...
Split the remaining space into /24 (5 free blocks)
  Subnets        248
```

### Aggregating, stepping and picking

`+<prefix>` answers "can these two be combined, and what does it cost?"

```
$ prefixtool 10.0.0.0/24 +10.0.3.0/24
...
Aggregate 10.0.0.0/24 with 10.0.3.0/24
  Smallest       10.0.0.0/22  (1,024 addresses, 1,022 usable)
  Also covers    2 blocks neither prefix uses
    10.0.1.0/24
    10.0.2.0/24
```

Inputs that fill the aggregate between them say so; one that already contains
the others is reported as such rather than pretending to combine anything.

Several `+` operators describe a single aggregate covering all of them, rather
than one pairing per operator:

```
$ prefixtool 10.0.0.0/24 +10.0.1.0/24 +10.1.0.0/16
...
Aggregate 10.0.0.0/24 with 10.0.1.0/24 and 10.1.0.0/16
  Smallest       10.0.0.0/15  (131,072 addresses, 131,070 usable)
  Also covers    7 blocks no input uses
```

`^N` walks along at the same size, which is what you want when handing out
blocks in order:

```
$ prefixtool 10.0.4.0/22 ^1 -q
10.0.8.0/22
```

`@N` is the inverse of `=`: rather than asking which subnet an address is in,
it asks for a subnet by number. Negative counts back from the end.

```
$ prefixtool 2001:db8::/52 /64 @3 @-1 -q -n 0
2001:db8:0:3::/64
2001:db8:0:fff::/64
```

### Reverse DNS zones

The `Reverse DNS` line says whether the prefix is a zone. Often it is not, and
then the question is which zones it actually is - the ones you have to go and
create. `.` answers that:

```
$ prefixtool 10.0.0.0/22 .
...
Reverse zones for 10.0.0.0/22
  Boundary       /24
  Zones          4

    0.0.10.in-addr.arpa.
    1.0.10.in-addr.arpa.
    2.0.10.in-addr.arpa.
    3.0.10.in-addr.arpa.
```

`in-addr.arpa` splits on octets and `ip6.arpa` on nibbles, so the boundary is
the next one at or below the prefix. `.N` cuts deeper instead, which is what
you want when handing zones out with the prefixes - `.56` on a `/48` gives the
256 zones the customer `/56`s need. Listings stay lazy, so `.64` on a `/32` is
four billion zones and still prints the first one at once.

An IPv4 prefix longer than a `/24` has no zone of its own, because the octet
below it is the last boundary there is. RFC 2317 delegates one anyway, by
pointing CNAMEs in the enclosing `/24` at a made-up sub-zone:

```
$ prefixtool 10.0.0.64/26 .
...
Reverse zones for 10.0.0.64/26
  Parent zone    0.0.10.in-addr.arpa.
  Delegation     64/26.0.0.10.in-addr.arpa.
  Note           longer than a /24, so it has no zone of its own: RFC 2317 has
                 0.0.10.in-addr.arpa. CNAME 64-127 into the delegated zone
```

Zone names are absolute, with the trailing dot, because that is what a zone
file or an `nsupdate` wants - a relative name is a different name once an
origin is in scope. Under `--quiet` they come out bare, one per line, so they
can be fed straight into whatever creates them.

### Locating an address

```
$ prefixtool 2001:db8::/52 /64 =2001:db8:0:3::5
...
Lookup 2001:db8:0:3::5
  yes - inside 2001:db8::/52
  /64 -> 2001:db8:0:3::/64   (subnet #3)
```

### Big numbers

Past 2^32 the exact digit count stops being something anyone reads, so the
report gives the width in bits and an order of magnitude instead:

```
  Addresses      2^76 (~7.6e22)
  Remaining      ~2^96 (~7.9e28) addresses in 32 blocks
```

A total that is not itself a power of two still has a width worth reading - a
/32 less a /64 is 2^96 for every practical purpose - so it is reported with a
tilde to mark the rounding.

`--json` is unaffected and still carries exact integers, so nothing is lost -
`jq .addresses` gives all 23 digits.

### Colour

The report is coloured when it is going to a terminal: the prefix under
inspection, section headings, granted allocations in green, refusals in red,
and anything you should not be assigning from in yellow.

It stays out of the way of everything else. Colour is off when the output is
piped or redirected, off for `--json` and `--quiet` whatever else you ask for,
and off when [`NO_COLOR`](https://no-color.org) is set or `TERM=dumb`.
`--color=always` forces it on, which is what you want for `| less -R`.

Styling never changes the layout - stripping the escape sequences from a
coloured report gives back the uncoloured one byte for byte, and a test holds
that.

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

## fabrictool

```
fabrictool [OPTIONS] <SWITCHES> [OP]...
```

`SWITCHES` is how many switches there are to connect. A bare count answers in
links, ports and cables; each operator adds something more it can say.

### Operators

| Operator | Meaning |
| --- | --- |
| `@SPEED` | The speed of each link: `@100G`, `@25G`, `@1.6T` |
| `%M` | Break each switch port into `M` lanes |
| `%SPEED` | The same, worked out from the port speed instead of counted |
| `xK`, `*K` | `K` parallel links between each pair |
| `/SHAPE` | `/mesh` (the default), `/ring`, `/star` or `/leaf-spine` |
| `+S` | `S` spines above the leaves, which makes it a leaf-spine (a pair, by default) |
| `-N@SPEED` | `N` server ports on each leaf, at that speed |
| `-N@SPEED%P` | The same, out of `P` ports split into lanes to reach them |
| `=N` | Each switch has `N` ports - does the plan fit? |
| `=N@SPEED` | The same, about the `N` ports it has at one speed |
| `.` | The patch schedule: which port on which switch reaches which |

A bare number after `%` is a lane count and a number with a unit is a port
speed, so `%4` and `%400G` are different questions and neither has to be
guessed at. Use the `x` form of a link count (`x2`) in `zsh`, which otherwise
tries to glob the `*`. Flags and operators can be given in any order.

### Options

| Flag | Meaning |
| --- | --- |
| `-n`, `--limit <N>` | Links to list in the patch schedule (default 8) |
| `-a`, `--all` | List every link, however many there are |
| `--media <kind>` | What the links are made of: `optic` (default), `aoc` or `dac` |
| `-q`, `--quiet` | Print the bill of materials only, one item per line |
| `--json` | Emit a JSON object instead of a report |
| `--color <when>` | `auto` (default), `always` or `never` |

### Exit status

| Code | Meaning |
| --- | --- |
| 0 | Success |
| 1 | Bad switch count or operator |
| 3 | The fabric cannot be built as asked |
| 4 | `--quiet`, and an `=N` port budget does not fit |

**3** is the code for a request that makes sense and still cannot be built - a
splitter cable with nothing at the far end to plug into, say. It is separate
from bad input for the same reason **4** is: a script should never read a typo
as a confident no.

### Counting a mesh

A full mesh is the shape whose cable count people get wrong, because it grows
with the square of the switch count rather than with the switch count. Eight
of them is twenty-eight links and fifty-six transceivers:

```
$ fabrictool 8 @100G
8 switches  -  full mesh at 100G

  Switches       8
  Topology       full mesh  (every switch to every other)
  Links          28  (1 link between each pair)
  Link speed     100G
  Per pair       100G
  Per switch     7 links, 700G
  Ports          7 x 100G on each switch
  Bisection      1.6T  (16 links across the middle)
  Fabric total   2.8T  (28 x 100G, one direction)
  Hops           1  (worst case, switch to switch)
  Resilience     6 links may fail before the fabric splits

Cabling 8 switches at 100G
  Arrangement    one 100G cable per link, a transceiver in each end

  Qty  Item                     Where
   56  100G transceiver         one in each end of every link
   28  duplex fibre patch lead  one per link
   56  100G switch port         7 ports on each of 8 switches
```

Everything after the first two lines is a consequence of the shape.
**Bisection** is what crosses the middle when the fabric is cut into halves,
which is the bandwidth available when the traffic is as awkward as it can be.
**Resilience** is how many links can fail before some switch cannot reach some
other. **Hops** is the worst case, switch to switch.

### Breaking a port out

Nobody buys 100G switches to build that mesh any more. They buy 400G ports and
split each one into four 100G lanes, each lane going to a different peer. `%`
says so, either as a lane count (`%4`) or as the port speed to work it out
from (`%400G`):

```
$ fabrictool 8 @100G %400G
8 switches  -  full mesh at 100G

  Switches       8
  Topology       full mesh  (every switch to every other)
  Links          28  (1 link between each pair)
  Link speed     100G
  Per pair       100G
  Per switch     7 links, 700G
  Ports          2 x 400G on each switch  (4 lanes each: 7 used, 1 spare)
  Bisection      1.6T  (16 links across the middle)
  Fabric total   2.8T  (28 x 100G, one direction)
  Hops           1  (worst case, switch to switch)
  Resilience     6 links may fail before the fabric splits

Cabling 8 switches at 100G
  Arrangement    400G ports split 4 ways at both ends, lanes joined in a patch field
  Trunk ports    16  (64 lanes, 56 used, 8 spare)

  Qty  Item                             Where
   16  400G transceiver                 one in each trunk port
   16  400G to 4x100G breakout harness  one per trunk port, its lanes fanned out to that many peers
   28  duplex coupler                   one per link, where its two lanes meet in the patch field
   16  400G switch port                 2 ports on each of 8 switches
```

The same twenty-eight links, but sixteen transceivers instead of fifty-six,
and two ports a switch instead of seven. The arrangement line is the part
worth agreeing with before trusting the rest: in a mesh every switch has the
same ports, so **both** ends of a link are lanes and they meet in a patch
field. The eight spare lanes are the price of seven peers not dividing by
four.

### Splitters, and where they can go

A DAC or AOC splitter is one assembly with its ends moulded on, so its lanes
have to land in ports that run at the lane speed. That is a star: the hub
breaks out, and each spoke gives up a whole port.

```
$ fabrictool 48 /star @25G %100G --media=dac
48 switches  -  star at 25G

  Switches       48
  Topology       star  (one hub, everything else hanging off it)
  Links          47  (1 link from the hub to each spoke)
  Link speed     25G
  Per spoke      25G
  Per switch     the hub 47 links, 1.175T
                 each spoke 1 link, 25G
  Ports          12 x 100G on the hub  (4 lanes each: 47 used, 1 spare)
                 1 x 25G on each spoke
  Bisection      600G  (24 links across the middle)
  Fabric total   1.175T  (47 x 25G, one direction)
  Hops           2  (worst case, switch to switch)
  Resilience     any single link failure splits the fabric

Cabling 48 switches at 25G
  Arrangement    the hub's 100G ports split 4 ways, a whole 25G port at each spoke
  Hub ports      12  (48 lanes, 47 used, 1 spare)

  Qty  Item                        Where
   12  100G to 4x25G DAC splitter  one per hub port, a lane to each of 4 spokes
   12  100G hub port               12 ports on the hub
   47  25G spoke port              1 port on each of 47 spokes
```

Ask for the same cable in a mesh and there is nothing for those ends to plug
into, because every switch has the same 400G ports. That is a fabric that
cannot be built rather than a typo, so it exits **3** and says what the two
ways round it are:

```
$ fabrictool 8 @100G %400G --media=dac; echo $?
fabrictool: a 4-lane DAC splitter ends in modules, and in a full mesh every
switch has the same ports, so there is nothing at 100G for those modules to
plug into. Use --media=optic and join the lanes in a patch field, or put the
splitter at one end only with /star or /leaf-spine
3
```

### Other shapes, and whether they fit

`/ring`, `/star` and `/leaf-spine` cost far fewer cables than a mesh, and the
report says what that buys and what it costs: a ring of twenty-four is two
links across the middle and twelve hops from one side to the other, however
fast each link is.

`=N` asks the question that decides whether any of this is orderable - the
switches have `N` ports, so does the plan fit in them?

```
$ fabrictool 48 /star @25G %100G =32
48 switches  -  star at 25G

  Switches       48
  Topology       star  (one hub, everything else hanging off it)
  Links          47  (1 link from the hub to each spoke)
  Link speed     25G
  Per spoke      25G
  Per switch     the hub 47 links, 1.175T
                 each spoke 1 link, 25G
  Ports          12 x 100G on the hub  (4 lanes each: 47 used, 1 spare)
                 1 x 25G on each spoke
  Bisection      600G  (24 links across the middle)
  Fabric total   1.175T  (47 x 25G, one direction)
  Hops           2  (worst case, switch to switch)
  Resilience     any single link failure splits the fabric

Cabling 48 switches at 25G
  Arrangement    the hub's 100G ports split 4 ways, a whole 25G port at each spoke
  Hub ports      12  (48 lanes, 47 used, 1 spare)

  Qty  Item                            Where
   12  100G transceiver                one in each hub port
   12  100G to 4x25G breakout harness  one per hub port, a lane to each spoke
   47  25G transceiver                 one in each spoke port
   12  100G hub port                   12 ports on the hub
   47  25G spoke port                  1 port on each of 47 spokes

Ports on a 32-port switch
  yes - it fits
  the hub     12 of 32 ports at 100G, 20 spare
  each spoke  1 of 32 ports at 25G, 31 spare
```

A star's hub and its spokes are different shapes, so they are answered
separately. The hub is the one that runs out.

### Leaf-spine, servers, and oversubscription

The shape most of this actually gets built in. `+2` puts two spines above the
leaves - saying how many spines there are is what makes it a leaf-spine, and a
pair is what `/leaf-spine` means on its own, because that is how a rack gets
built. `-48@25G` says what is plugged into each leaf:

```
$ fabrictool 16 +2 @100G %400G -48@25G
16 leaves + 2 spines  -  leaf-spine at 100G

  Switches       18  (16 leaves, 2 spines)
  Topology       leaf-spine  (every leaf to every spine)
  Links          32  (1 link from each leaf to each spine)
  Link speed     100G
  Per uplink     100G
  Per switch     each spine 16 links, 1.6T
                 each leaf 2 links, 200G
  Ports          4 x 400G on each spine  (4 lanes each: 16 used, 0 spare)
                 2 x 100G on each leaf
  Server ports   48 x 25G on each leaf
  Bisection      1.6T  (16 links across the middle)
  Fabric total   3.2T  (32 x 100G, one direction)
  Hops           2  (worst case, switch to switch)
  Resilience     1 link may fail before the fabric splits
  Spine loss     each leaf keeps 1 uplink of 2  (100G of 200G)

Cabling 16 leaves + 2 spines at 100G
  Arrangement    each spine's 400G ports split 4 ways, a whole 100G port at each leaf
  Spine ports    8  (32 lanes, 32 used, 0 spare)

  Qty  Item                             Where
    8  400G transceiver                 one in each spine port
    8  400G to 4x100G breakout harness  one per spine port, a lane to each leaf
   32  100G transceiver                 one in each leaf port
    8  400G spine port                  4 ports on each of 2 spines
   32  100G leaf port                   2 ports on each of 16 leaves
  768  25G leaf server port             48 ports facing 48 servers on each of 16 leaves

Oversubscription
  Per leaf       6:1  (1.2T attached against 200G of fabric)
  One spine down 12:1  (100G of fabric left on each leaf)
  Servers        768 x 25G  (19.2T attached in total)
```

Read it from the bottom. Each leaf has 48 servers at 25G attached, which is
1.2T wanting to leave, and two 100G uplinks to leave through, which is 200G:
**6:1**. The fix is more uplinks, and the ratio is the reason to buy them:

```
$ for s in +2 +4 +6 +12; do fabrictool 16 $s @100G -48@25G | grep 'Per leaf'; done
  Per leaf       6:1  (1.2T attached against 200G of fabric)
  Per leaf       3:1  (1.2T attached against 400G of fabric)
  Per leaf       2:1  (1.2T attached against 600G of fabric)
  Per leaf       1:1  (1.2T attached against 1.2T of fabric - non-blocking)
```

**Spine loss** is the other half of buying a pair, and it is a line of its own:
a leaf keeps one of its two uplinks when a spine goes, so it stays up and the
ratio doubles. That is the number to check before deciding two spines is
enough - 6:1 is a design, 12:1 during a spine reload is a decision.

One spine is allowed and says what it is:

```
$ fabrictool 16 +1 @100G -48@25G | grep Caution
  Caution        one spine is a single point of failure; a pair is the usual build
```

A rack is often just a pair of leaves under that pair of spines, which is four
links and every leaf on every spine:

```
$ fabrictool 2 +2 @100G -48@25G
2 leaves + 2 spines  -  leaf-spine at 100G

  Switches       4  (2 leaves, 2 spines)
  Topology       leaf-spine  (every leaf to every spine)
  Links          4  (1 link from each leaf to each spine)
  Link speed     100G
  Per uplink     100G
  Per switch     each spine 2 links, 200G
                 each leaf 2 links, 200G
  Ports          2 x 100G on each spine
                 2 x 100G on each leaf
  Server ports   48 x 25G on each leaf
  Bisection      200G  (2 links across the middle)
  Fabric total   400G  (4 x 100G, one direction)
  Hops           2  (worst case, switch to switch)
  Resilience     1 link may fail before the fabric splits
  Spine loss     each leaf keeps 1 uplink of 2  (100G of 200G)

Cabling 2 leaves + 2 spines at 100G
  Arrangement    one 100G cable per link, a transceiver in each end

  Qty  Item                     Where
    8  100G transceiver         one in each end of every link
    4  duplex fibre patch lead  one per link
    4  100G spine port          2 ports on each of 2 spines
    4  100G leaf port           2 ports on each of 2 leaves
   96  25G leaf server port     48 ports facing 48 servers on each of 2 leaves

Oversubscription
  Per leaf       6:1  (1.2T attached against 200G of fabric)
  One spine down 12:1  (100G of fabric left on each leaf)
  Servers        96 x 25G  (2.4T attached in total)
```

The spine side is the splitter arrangement again, and this is where it earns
its keep: a 400G spine port fans out to four leaves, so thirty-two leaves off
four spines is eight ports a spine rather than thirty-two.

Servers come out of the same front panel as the uplinks, so `=N` counts both:

```
$ fabrictool 32 +4 @100G %400G -48@25G =56
32 leaves + 4 spines  -  leaf-spine at 100G

  Switches       36  (32 leaves, 4 spines)
  Topology       leaf-spine  (every leaf to every spine)
  Links          128  (1 link from each leaf to each spine)
  Link speed     100G
  Per uplink     100G
  Per switch     each spine 32 links, 3.2T
                 each leaf 4 links, 400G
  Ports          8 x 400G on each spine  (4 lanes each: 32 used, 0 spare)
                 4 x 100G on each leaf
  Server ports   48 x 25G on each leaf
  Bisection      6.4T  (64 links across the middle)
  Fabric total   12.8T  (128 x 100G, one direction)
  Hops           2  (worst case, switch to switch)
  Resilience     3 links may fail before the fabric splits
  Spine loss     each leaf keeps 3 uplinks of 4  (300G of 400G)

Cabling 32 leaves + 4 spines at 100G
  Arrangement    each spine's 400G ports split 4 ways, a whole 100G port at each leaf
  Spine ports    32  (128 lanes, 128 used, 0 spare)

    Qty  Item                             Where
     32  400G transceiver                 one in each spine port
     32  400G to 4x100G breakout harness  one per spine port, a lane to each leaf
    128  100G transceiver                 one in each leaf port
     32  400G spine port                  8 ports on each of 4 spines
    128  100G leaf port                   4 ports on each of 32 leaves
  1,536  25G leaf server port             48 ports facing 48 servers on each of 32 leaves

Oversubscription
  Per leaf       3:1  (1.2T attached against 400G of fabric)
  One spine down 4:1  (300G of fabric left on each leaf)
  Servers        1,536 x 25G  (38.4T attached in total)

Ports on a 56-port switch
  yes - it fits
  each spine  8 of 56 ports at 400G, 48 spare
  each leaf   52 of 56 ports (4 at 100G, 48 at 25G), 4 spare
```

A leaf whose servers arrive on split ports is spelled `-48@25G%100G` - 48
servers at 25G out of twelve 100G ports broken four ways - which changes what
the ports cost without changing the ratio, since the ratio is about the
servers rather than the ports they arrive on.

### A real build, four ways

Four 32-port 400G spines, and four racks with a pair of leaf switches each -
eight leaves. Each leaf is a 48x25G box with two 200G ports and four 100G
ports, and the full mesh between leaf and spine has to hold: every leaf on
every spine, which is 32 links however they are made.

The leaf's front panel is the constraint, so state it once and reuse it.
`=N@SPEED` asks about the ports at one speed, which is how the switch is
actually specified:

```
profile="=32@400G =4@100G =2@200G =48@25G"
```

**One 100G link per spine port.** The simplest cabling: a 100G transceiver at
each end and a fibre between them, no breakout and no patch field. It costs
spine ports - eight of each spine's 32, one per leaf - and 64 transceivers:

```
$ fabrictool 8 +4 @100G -48@25G =32@400G =4@100G =2@200G =48@25G
8 leaves + 4 spines  -  leaf-spine at 100G

  Switches       12  (8 leaves, 4 spines)
  Topology       leaf-spine  (every leaf to every spine)
  Links          32  (1 link from each leaf to each spine)
  Link speed     100G
  Per uplink     100G
  Per switch     each spine 8 links, 800G
                 each leaf 4 links, 400G
  Ports          8 x 100G on each spine
                 4 x 100G on each leaf
  Server ports   48 x 25G on each leaf
  Bisection      1.6T  (16 links across the middle)
  Fabric total   3.2T  (32 x 100G, one direction)
  Hops           2  (worst case, switch to switch)
  Resilience     3 links may fail before the fabric splits
  Spine loss     each leaf keeps 3 uplinks of 4  (300G of 400G)

Cabling 8 leaves + 4 spines at 100G
  Arrangement    one 100G cable per link, a transceiver in each end

  Qty  Item                     Where
   64  100G transceiver         one in each end of every link
   32  duplex fibre patch lead  one per link
   32  100G spine port          8 ports on each of 4 spines
   32  100G leaf port           4 ports on each of 8 leaves
  384  25G leaf server port     48 ports facing 48 servers on each of 8 leaves

Oversubscription
  Per leaf       3:1  (1.2T attached against 400G of fabric)
  One spine down 4:1  (300G of fabric left on each leaf)
  Servers        384 x 25G  (9.6T attached in total)

400G ports on a 32-port switch
  yes - nothing in this plan uses a port at 400G

100G ports on a 4-port switch
  no - each spine is 4 short
  each spine  8 of 4 ports at 100G, 4 short
  each leaf   4 of 4 ports at 100G, 0 spare

200G ports on a 2-port switch
  yes - nothing in this plan uses a port at 200G

25G ports on a 48-port switch
  yes - it fits
  each leaf  48 of 48 ports at 25G, 0 spare
```

**Split each spine port into 4x100G.** The same 32 links, out of two spine
ports per spine instead of eight. With optics that is a 400G transceiver and a
breakout harness per spine port, plus a 100G transceiver at each leaf:

```
$ fabrictool 8 +4 @100G %400G -48@25G =32@400G =4@100G =2@200G =48@25G
8 leaves + 4 spines  -  leaf-spine at 100G

  Switches       12  (8 leaves, 4 spines)
  Topology       leaf-spine  (every leaf to every spine)
  Links          32  (1 link from each leaf to each spine)
  Link speed     100G
  Per uplink     100G
  Per switch     each spine 8 links, 800G
                 each leaf 4 links, 400G
  Ports          2 x 400G on each spine  (4 lanes each: 8 used, 0 spare)
                 4 x 100G on each leaf
  Server ports   48 x 25G on each leaf
  Bisection      1.6T  (16 links across the middle)
  Fabric total   3.2T  (32 x 100G, one direction)
  Hops           2  (worst case, switch to switch)
  Resilience     3 links may fail before the fabric splits
  Spine loss     each leaf keeps 3 uplinks of 4  (300G of 400G)

Cabling 8 leaves + 4 spines at 100G
  Arrangement    each spine's 400G ports split 4 ways, a whole 100G port at each leaf
  Spine ports    8  (32 lanes, 32 used, 0 spare)

  Qty  Item                             Where
    8  400G transceiver                 one in each spine port
    8  400G to 4x100G breakout harness  one per spine port, a lane to each leaf
   32  100G transceiver                 one in each leaf port
    8  400G spine port                  2 ports on each of 4 spines
   32  100G leaf port                   4 ports on each of 8 leaves
  384  25G leaf server port             48 ports facing 48 servers on each of 8 leaves

Oversubscription
  Per leaf       3:1  (1.2T attached against 400G of fabric)
  One spine down 4:1  (300G of fabric left on each leaf)
  Servers        384 x 25G  (9.6T attached in total)

400G ports on a 32-port switch
  yes - it fits
  each spine  2 of 32 ports at 400G, 30 spare

100G ports on a 4-port switch
  yes - it fits
  each leaf  4 of 4 ports at 100G, 0 spare

200G ports on a 2-port switch
  yes - nothing in this plan uses a port at 200G

25G ports on a 48-port switch
  yes - it fits
  each leaf  48 of 48 ports at 25G, 0 spare
```

**The same, as AOCs.** A 400G-to-4x100G active optical splitter arrives as one
assembly with its ends attached, so the whole spine side is eight cables and
no transceivers at all:

```
$ fabrictool 8 +4 @100G %400G -48@25G --media=aoc
8 leaves + 4 spines  -  leaf-spine at 100G

  Switches       12  (8 leaves, 4 spines)
  Topology       leaf-spine  (every leaf to every spine)
  Links          32  (1 link from each leaf to each spine)
  Link speed     100G
  Per uplink     100G
  Per switch     each spine 8 links, 800G
                 each leaf 4 links, 400G
  Ports          2 x 400G on each spine  (4 lanes each: 8 used, 0 spare)
                 4 x 100G on each leaf
  Server ports   48 x 25G on each leaf
  Bisection      1.6T  (16 links across the middle)
  Fabric total   3.2T  (32 x 100G, one direction)
  Hops           2  (worst case, switch to switch)
  Resilience     3 links may fail before the fabric splits
  Spine loss     each leaf keeps 3 uplinks of 4  (300G of 400G)

Cabling 8 leaves + 4 spines at 100G
  Arrangement    each spine's 400G ports split 4 ways, a whole 100G port at each leaf
  Spine ports    8  (32 lanes, 32 used, 0 spare)

  Qty  Item                         Where
    8  400G to 4x100G AOC splitter  one per spine port, a lane to each of 4 leaves
    8  400G spine port              2 ports on each of 4 spines
   32  100G leaf port               4 ports on each of 8 leaves
  384  25G leaf server port         48 ports facing 48 servers on each of 8 leaves

Oversubscription
  Per leaf       3:1  (1.2T attached against 400G of fabric)
  One spine down 4:1  (300G of fabric left on each leaf)
  Servers        384 x 25G  (9.6T attached in total)
```

**2x200G instead.** A 400G port splits two ways as readily as four, and 200G
links would double the fabric each leaf has - but the mesh has to reach four
spines, and a leaf with two 200G ports can only reach two of them:

```
$ fabrictool 8 +4 @200G %400G -48@25G =32@400G =4@100G =2@200G =48@25G
8 leaves + 4 spines  -  leaf-spine at 200G

  Switches       12  (8 leaves, 4 spines)
  Topology       leaf-spine  (every leaf to every spine)
  Links          32  (1 link from each leaf to each spine)
  Link speed     200G
  Per uplink     200G
  Per switch     each spine 8 links, 1.6T
                 each leaf 4 links, 800G
  Ports          4 x 400G on each spine  (2 lanes each: 8 used, 0 spare)
                 4 x 200G on each leaf
  Server ports   48 x 25G on each leaf
  Bisection      3.2T  (16 links across the middle)
  Fabric total   6.4T  (32 x 200G, one direction)
  Hops           2  (worst case, switch to switch)
  Resilience     3 links may fail before the fabric splits
  Spine loss     each leaf keeps 3 uplinks of 4  (600G of 800G)

Cabling 8 leaves + 4 spines at 200G
  Arrangement    each spine's 400G ports split 2 ways, a whole 200G port at each leaf
  Spine ports    16  (32 lanes, 32 used, 0 spare)

  Qty  Item                             Where
   16  400G transceiver                 one in each spine port
   16  400G to 2x200G breakout harness  one per spine port, a lane to each leaf
   32  200G transceiver                 one in each leaf port
   16  400G spine port                  4 ports on each of 4 spines
   32  200G leaf port                   4 ports on each of 8 leaves
  384  25G leaf server port             48 ports facing 48 servers on each of 8 leaves

Oversubscription
  Per leaf       1.5:1  (1.2T attached against 800G of fabric)
  One spine down 2:1  (600G of fabric left on each leaf)
  Servers        384 x 25G  (9.6T attached in total)

400G ports on a 32-port switch
  yes - it fits
  each spine  4 of 32 ports at 400G, 28 spare

100G ports on a 4-port switch
  yes - nothing in this plan uses a port at 100G

200G ports on a 2-port switch
  no - each leaf is 2 short
  each leaf  4 of 2 ports at 200G, 2 short

25G ports on a 48-port switch
  yes - it fits
  each leaf  48 of 48 ports at 25G, 0 spare
```

That is the answer: **200G to every spine does not fit these leaves**, and the
tool says which port ran out rather than leaving it to be noticed. Under
`--quiet` that is exit 4, so a script comparing options can just ask.

What the numbers come to, for the three that do fit:

| | Spine ports | Spine optics | Leaf optics | Cables | Per leaf |
| --- | --- | --- | --- | --- | --- |
| 100G straight | 8 of 32 | 32 x 100G | 32 x 100G | 32 fibre leads | 4x100G, 3:1 |
| 400G split 4x100G, optics | 2 of 32 | 8 x 400G | 32 x 100G | 8 harnesses | 4x100G, 3:1 |
| 400G split 4x100G, AOC | 2 of 32 | none | none | 8 AOC splitters | 4x100G, 3:1 |

All three keep the full mesh and land on the same 3:1 oversubscription,
because that is set by the leaf - 48x25G attached against 4x100G of uplink -
and not by how the uplinks are cabled. What differs is spine ports and parts:
the breakout options free six ports on every spine, and the AOC option removes
the transceiver count entirely at the cost of fixed-length assemblies.

A mixed fabric - two spines reached at 200G and two at 100G, using both kinds
of leaf port - is outside what one run models, because a switch here has one
port speed facing the fabric. Two runs answer it, since the spines are
separate devices either way:

```
$ fabrictool 8 +2 @200G %400G -48@25G | grep -E 'Per leaf|Spine ports'
$ fabrictool 8 +2 @100G %400G -48@25G | grep -E 'Per leaf|Spine ports'
```

Add the spine ports; the leaf then uses both 200G ports and two of its four
100G ports, and the ratio is what the two runs' fabric totals come to
together.

### The patch schedule

`.` prints what to take to the rack, in `sw<switch>:<port>/<lane>`:

```
$ fabrictool 8 @100G %400G . -n 6
8 switches  -  full mesh at 100G

  Switches       8
  Topology       full mesh  (every switch to every other)
  Links          28  (1 link between each pair)
  Link speed     100G
  Per pair       100G
  Per switch     7 links, 700G
  Ports          2 x 400G on each switch  (4 lanes each: 7 used, 1 spare)
  Bisection      1.6T  (16 links across the middle)
  Fabric total   2.8T  (28 x 100G, one direction)
  Hops           1  (worst case, switch to switch)
  Resilience     6 links may fail before the fabric splits

Cabling 8 switches at 100G
  Arrangement    400G ports split 4 ways at both ends, lanes joined in a patch field
  Trunk ports    16  (64 lanes, 56 used, 8 spare)

  Qty  Item                             Where
   16  400G transceiver                 one in each trunk port
   16  400G to 4x100G breakout harness  one per trunk port, its lanes fanned out to that many peers
   28  duplex coupler                   one per link, where its two lanes meet in the patch field
   16  400G switch port                 2 ports on each of 8 switches

Patch schedule for 8 switches
  Notation       sw<switch>:<port>/<lane>

    sw1:1/1  ->  sw2:1/1
    sw1:1/2  ->  sw3:1/1
    sw1:1/3  ->  sw4:1/1
    sw1:1/4  ->  sw5:1/1
    sw1:2/1  ->  sw6:1/1
    sw1:2/2  ->  sw7:1/1
    ... (showing 6 of 28; use --all or -n N)
```

Lanes fill a port before the next port is used, and the order is fixed, so two
people cabling from the same schedule wire the same fabric. `--all` prints
every link, and the list stays lazy - piping a large one into `head` returns
immediately.

### Scripting

`--quiet` prints the bill of materials, one item per line, as a quantity and a
stable name with a tab between them:

```
$ fabrictool 8 @100G %400G -q
16	transceiver-400G
16	breakout-1x4-400G
28	coupler-100G
16	port-switch-400G
```

With `.` it prints the schedule instead, one link per line, since that is what
was asked for:

```
$ fabrictool 4 . -q
sw1:1 sw2:1
sw1:2 sw3:1
sw1:3 sw4:1
sw2:2 sw3:2
sw2:3 sw4:2
sw3:3 sw4:3
```

`--json` emits everything the report knows, with every rate in megabits per
second as an exact integer:

```
$ fabrictool 8 @100G %400G --json | jq '{links, trunk_ports, bisection: .bandwidth.bisection_mbps}'
{
  "links": 28,
  "trunk_ports": 16,
  "bisection": 1600000
}
```

And under `--quiet` an `=N` is a question, so the exit status answers it:

```
if fabrictool 32 @100G %400G =32 -q > /dev/null; then
    echo "it fits"
fi
```

## Releasing

Releases are cut by merging a pull request, not by pushing a tag by hand:

1. Bump `version` in `Cargo.toml` on a branch and open a PR.
2. Get it approved and merge it.
3. Landing on the default branch creates the matching `v<version>` tag and runs
   the release, which builds all six targets and publishes them.

Approving the PR is the act that publishes, so nothing reaches the releases
page without a review. A merge that does not change the version is ignored
(Dependabot's manifest updates included), and a version whose tag already
exists is left alone, so re-running is harmless.

Versions below `1.0.0`, and any version with a suffix such as `1.0.0-rc1`, are
published as pre-releases.

For a second, explicit approval before the tag is created, add required
reviewers to the `release` environment under Settings -> Environments. Without
that the environment imposes no gate.

Every archive carries both binaries, and the release smoke-tests both before
publishing anything.

The release also regenerates `Formula/prefixtool.rb` from the archives it just
built and commits it to the default branch, so the Homebrew formula never
lags behind a release. That job runs after the release is published, so if it
cannot push, the release still stands and only the formula is stale.

## Tests

```
cargo test
```

Unit tests cover the allocator, the operator grammar, the special-range table,
the reverse-DNS zones, the big-number formatting, and, on the fabric side, the
topology arithmetic, the rate parsing and the patch schedule. `tests/cli.rs`
and `tests/fabric.rs` run the real binaries and check their output and exit
codes.

Several of the more valuable tests are properties rather than examples: a
carve's blocks tile their parent exactly, an aggregate contains every input,
and a fabric's link ends, ports and schedule all describe the same fabric.

## License

MIT - see [LICENSE](LICENSE).
