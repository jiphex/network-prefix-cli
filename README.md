# prefixtool

[![CI](https://github.com/jiphex/network-prefix-cli/actions/workflows/ci.yml/badge.svg)](https://github.com/jiphex/network-prefix-cli/actions/workflows/ci.yml)

A single-binary CLI for inspecting, splitting and carving up IPv4 and IPv6
prefixes. Built for the moment you are staring at an allocation and need to
know how it divides, what fits inside it, and what is left over afterwards.

## Install

Grab a binary for your platform from the
[releases page](https://github.com/jiphex/network-prefix-cli/releases), unpack
it and put `prefixtool` on your `PATH`. Each archive ships with a `.sha256`
next to it. Builds are published for Linux (x86-64 gnu and static musl,
arm64), macOS (Intel and Apple silicon) and Windows.

### Homebrew

```
brew tap jiphex/network-prefix-cli https://github.com/jiphex/network-prefix-cli
brew install jiphex/network-prefix-cli/prefixtool
```

The formula lives in this repository under `Formula/`, so the tap needs the
repository URL spelled out - Homebrew otherwise goes looking for a repository
called `homebrew-network-prefix-cli`. It installs the prebuilt binary for your
platform, so there is no Rust toolchain and no compile.

Homebrew clears the quarantine flag itself, so the macOS note below does not
apply to a `brew install`.

### macOS

The macOS binaries are ad-hoc signed but not notarized, so Gatekeeper will
object to one carrying a quarantine flag. Unpacking from Terminal never sets
that flag:

```
tar xzf prefixtool-<tag>-aarch64-apple-darwin.tar.gz
./prefixtool --version
```

If you downloaded through a browser and hit *"Apple could not verify..."*, the
macOS archives bundle a script for it:

```
./macos-unquarantine.sh
```

It clears the quarantine flag, repairs the ad-hoc signature if it needs it, and
runs the binary to prove the result works. The equivalent by hand is
`xattr -d com.apple.quarantine prefixtool`.

Or build it yourself:

```
cargo install --git https://github.com/jiphex/network-prefix-cli
```

From a checkout:

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
| `%M` | Split it into `M` subnets, whatever lengths that needs |
| `-N` | Carve one `/N` out of the prefix |
| `-N*K`, `-NxK` | Carve `K` subnets of `/N` |
| `-<prefix>` | Reserve one specific subnet, wherever it sits |
| `+N` | Show the enclosing `/N` supernet |
| `+<prefix>` | Aggregate; several `+` make one aggregate covering them all |
| `=<addr\|prefix>` | Ask whether an address or prefix falls inside |
| `@N` | The Nth subnet of a requested split; `@-1` is the last |
| `^N` | The prefix `N` blocks along at the same size; `^-1` is the previous |

Use the `x` form of a count (`-64x2`) in `zsh`, which otherwise tries to glob
the `*`. Flags and operators can be given in any order.

### Options

| Flag | Meaning |
| --- | --- |
| `-n`, `--limit <N>` | Prefixes to list per section (default 8) |
| `-a`, `--all` | List every prefix, however many there are |
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

## What it tells you

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

The release also regenerates `Formula/prefixtool.rb` from the archives it just
built and commits it to the default branch, so the Homebrew formula never
lags behind a release. That job runs after the release is published, so if it
cannot push, the release still stands and only the formula is stale.

## Tests

```
cargo test
```

Unit tests cover the allocator, the operator grammar, the special-range table,
the reverse-DNS zones and the big-number formatting; `tests/cli.rs` runs the
real binary and checks its output and exit codes.

## License

MIT - see [LICENSE](LICENSE).
