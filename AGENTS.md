# Working on prefixtool

A CLI that inspects, splits and carves IPv4 and IPv6 prefixes. The README
covers what it does; this covers how to change it without breaking things that
are easy to break here.

## Commands

```
cargo build
cargo test --locked --all-targets      # 179 tests: 118 unit, 61 end-to-end
cargo clippy --locked --all-targets
cargo fmt --all --check
```

CI runs all four with `RUSTFLAGS=-D warnings` on Linux, macOS and Windows, so
run them that way before pushing. `--locked` means **`Cargo.lock` has to be
refreshed whenever the version changes** - it records the package's own
version, and a stale lockfile fails the build rather than quietly updating.

Run the suite through a pty as well as a pipe when touching anything that
looks at its environment:

```
script -qec "cargo test --locked --all-targets" /dev/null
```

## Dependencies

`ipnet`, `clap`, `nom`. That is the whole list and it is deliberate.

The colour handling, the JSON writer, the big-number formatting and the
allocator are all hand-rolled because each would otherwise be a dependency
earning its keep only in a corner of one module. Terminal detection uses
`std::io::IsTerminal`. Adding a crate is a decision to raise with whoever owns
the repository, not a detail.

## Layout

| Module | Holds |
| --- | --- |
| `ops.rs` | The operator grammar, parsed with nom |
| `report.rs` | Turns a prefix plus operators into everything to be shown |
| `carve.rs` | The best-fit allocator, and the map of a parent's blocks |
| `render.rs` | Text, `--quiet` and `--json` output |
| `num.rs` | Address counts as powers of two, and how they are written |
| `style.rs` | Terminal colour |
| `info.rs`, `wellknown.rs` | Facts about a single prefix |
| `zones.rs` | Reverse DNS delegation zones, including RFC 2317 |
| `json.rs` | A small JSON writer |

## Conventions the tests enforce

**Pad before styling.** An escape sequence has no printed width, but
`format!("{:<width$}")` counts it anyway, so styling a value before padding it
silently shifts every column after it - and looks fine in a plain-text test.
`colour_never_changes_the_layout` asserts that stripping the escapes from a
coloured report gives back the uncoloured one byte for byte. Extend it when
adding a section.

**Tests must not depend on their environment.** A colour test once asked the
real stdout whether it was a terminal and asserted the answer was no. It
passed under a pipe, which is how CI runs, and failed under a terminal, which
is how an interactive `cargo test` and a Nix builder run. Hand such decisions
in as arguments and test the rule, not the machine - see `Style::decide`.

**The report summarises, the JSON does not.** Past 2^32 the human report gives
a power of two and an order of magnitude; `--json` always carries exact
integers. `num::describe_sum` is for people, `num::sum_grouped` for machines.
Indexes stay exact everywhere: an approximated index is a wrong answer rather
than a rounded one.

**`--quiet` and `--json` are for parsing.** Never coloured, whatever `--color`
says. No truncation hints, no prose. `list` returns whether more was waiting so
that the human renderer can say so and the machine ones can ignore it.

One prefix per line, with one exception: a `%a:b:c` ratio prints one line per
share, space-separated, because a share can be several blocks and nothing
about the blocks says how many. Those lines ignore `-n` - truncating one is a
wrong answer rather than a short one - and a ratio suppresses the free-block
list for the space it describes, as a split already does, so its lines cannot
be mistaken for single-block shares.

**Listings stay lazy.** `--all` on `::/0 /128` must return immediately when
piped to `head`. Do not collect a split into a `Vec`.

## Invariants worth keeping

Two of the more valuable tests are properties rather than examples, and both
found real bugs when first written:

- The carve map's rows **tile the parent exactly** - abutting, no gap, no
  overlap. Writing this exposed `Request::Floating` carrying a count it could
  allocate but only partly report.
- An aggregate **contains every input, and no spare block overlaps one**.
  Writing this exposed `+` aggregating pairwise, which listed a prefix the
  user had named as unused space.
- A `%a:b:c` ratio's blocks **tile the space exactly**, over a ragged
  remainder as well as a whole prefix, and `%1:1:...:1` produces the same
  sizes as `%M`. The second one is what pins the rounding rule: a ratio is
  rounded the same way a count already is, so there is one rule to explain
  rather than two.
- Filling from either end gives **mirror images**: `--from=top` reflects each
  allocation about the middle of the parent. Both the block chosen and the
  half taken when splitting down to it have to flip, and a test that only
  checked one of those passes while allocations land at the wrong end.

When adding an operator, reach for the property first.

A `%a:b:c` ratio can be inexact for two unrelated reasons, and the report has
to say which: the ratio itself may not be cuttable from any prefix (`2:1` -
two thirds of a prefix is not a prefix), or the ratio may be fine and the
space no longer a single block. `Shares::ratio_is_dyadic` is the test that
separates them.

## Arithmetic traps

IPv6 sizes overflow the obvious types. Two cases have bitten and are covered:

- Stepping a `/1` needs a block size of `2^127`, which is `i128::MAX + 1`.
  Compute offsets unsigned in both directions.
- `@-1` over `::/0` split into `/128`s needs index `2^128-1` against a count of
  `2^128`, which does not fit in a `u128` at all. Count back from the top
  instead of computing the count.
- Sharing works in units of the smallest free block, so a ragged remainder
  forces a very fine unit and the counts get large. They stay inside a `u128`
  only because the doubling loop runs solely for a whole prefix, which starts
  at one unit and stops as soon as it has enough. Do not widen its condition
  without re-checking that.

`num::Count` holds an exponent rather than a value for this reason.

## Shell-facing details

Operator sigils must survive an unquoted shell. `*` is a glob, which is why
`-64x2` exists alongside `-64*2`; `>` and `<` are redirection, which is why
stepping is `^N`. `@`, `^`, `%`, `+`, `=`, `/`, `.` and `:` are all safe in
bash and zsh. `~` is not, despite looking free: `~1` is directory-stack
expansion in both shells.

A new sigil has to be added to `looks_like_op` as well as to the grammar, or
it will not survive being interleaved with flags.

`:` is doing double duty, as the separator in `%a:b:c` and as the start of a
carve's name, and IPv6 addresses are mostly colons. Two rules keep it
unambiguous, and both have tests: a prefix length never contains a colon, so
the *first* one starts a name there; and `-<prefix>` tries the whole payload
as an address before splitting anything off, so `-2001:db8::1` stays an
address rather than becoming `2001:db8:` named `1`.

Flags and operators may be interleaved. `arrange()` in `main.rs` partitions
argv before clap sees it, because clap would otherwise swallow `--json` into
the operator list.

## Releasing

`Cargo.toml` is the only place a version lives. The flake reads it with
`fromTOML`; the Homebrew formula is generated from it.

1. Bump `version` in `Cargo.toml` on a branch, refresh `Cargo.lock`, open a PR.
2. Merging it to the default branch creates the `v<version>` tag and publishes
   the release: six targets built, then the formula regenerated and committed.

Do not create tags by hand - merging the bump is what cuts a release, and an
agent may not have permission to push tags anyway. A merge that does not change
the version is ignored, so Dependabot's manifest updates are safe.

Versions below `1.0.0`, and any with a suffix, publish as pre-releases.

## Documentation

README examples are generated from the binary and diffed against it, not
written by hand. After changing output, regenerate the affected block and
confirm it matches:

```
diff <(./target/debug/prefixtool 2001::/64 --color=never) \
     <(sed -n '/^\$ prefixtool 2001::\/64$/,/^```$/p' README.md | sed '1d;$d')
```
