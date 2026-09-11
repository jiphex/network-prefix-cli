# Working on prefixtool and fabrictool

This package builds two CLIs. `prefixtool` inspects, splits and carves IPv4
and IPv6 prefixes, and `fabrictool` sizes the cabling between switches. The README
covers what they do; this covers how to change them without breaking things
that are easy to break here.

They share `src/lib.rs` and nothing else. What is shared is how output looks -
colour, the JSON writer, digit grouping - and that is deliberate: the two
reports are meant to sit in the same terminal without looking like different
programs, so a change to one of those modules is a change to both tools.

## Commands

```
cargo build
cargo test --locked --all-targets      # unit, CLI and end-to-end tests
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

The dependencies are `ipnet`, `clap` and `nom`. That is the whole list, and
it is deliberate.

The colour handling, the JSON writer, the big-number formatting and the
allocator are all hand-rolled because each would otherwise be a dependency
earning its keep only in a corner of one module. Terminal detection uses
`std::io::IsTerminal`. Adding a crate is a decision to raise with whoever owns
the repository, not a detail.

## Layout

`src/main.rs` is prefixtool and `src/bin/fabrictool.rs` is fabrictool; both are
thin, and everything either of them does lives in `src/lib.rs` behind them.

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

The fabric side lives under `src/fabric/` and is the same shape one level
down:

| Module | Holds |
| --- | --- |
| `fabric/speed.rs` | Port and link rates, held in megabits per second |
| `fabric/ops.rs` | The operator grammar, hand-written |
| `fabric/plan.rs` | Topology, ports, cables, transceivers and bandwidth |
| `fabric/schedule.rs` | Which port on which switch reaches which |
| `fabric/render.rs` | Text, `--quiet` and `--json` output |
| `fabric/dot.rs` | A Graphviz drawing of the fabric |

`num.rs`, `style.rs` and `json.rs` are the three modules both tools use. The
fabric grammar is hand-written rather than parsed with nom because it has no
ambiguity to resolve: every operator is a sigil and a payload whose shape the
sigil chooses. Reach for nom there only if that stops being true.

## Conventions the tests enforce

**Pad before styling.** An escape sequence has no printed width, but
`format!("{:<width$}")` counts it anyway, so styling a value before padding it
silently shifts every column after it - and looks fine in a plain-text test.
`colour_never_changes_the_layout` asserts that stripping the escapes from a
coloured report gives back the uncoloured one byte for byte. There is one of
these per tool, and both matter. Extend the right one when adding a section.

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

**`--quiet` and `--json` are for parsing.** They are never coloured, whatever
`--color` says, and they carry no truncation hints and no prose. `list` returns whether more was waiting so
that the human renderer can say so and the machine ones can ignore it.

They print one prefix per line, with one exception. A `%a:b:c` ratio prints
one line per share, space-separated, because a share can be several blocks and
nothing about the blocks says how many. Those lines ignore `-n` - truncating one is a
wrong answer rather than a short one.

fabrictool's `--quiet` is a bill of materials, `quantity<TAB>item`, or the
patch schedule when `--schedule` asked for one - never both at once. Two shapes of line
in one stream is worse than either, and a reader piping it wants one of them.
The item names are an interface: `transceiver-400G` and `breakout-1x4-400G`
are what a script greps for, so treat renaming one as a breaking change.

**Nothing prints the same addresses twice.** A carve lists what it left over,
but `/N`, `%M` and `%a:b:c` each describe that same space themselves, so any
of them suppresses the free-block list. Leaving it in prints the remainder at
two granularities at once - and for a ratio the bare free lines read as
single-block shares. Anything new that divides the remainder belongs in that
condition in `quiet`.

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
  checked one of those passes while allocations sit at the wrong end.

When adding an operator, reach for the property first.

The fabric side has three of its own, and the first two found real bugs:

- Every link has two ends, and each end sits on exactly one switch: the sides'
  degrees times their switch counts is **twice the link count**, in every
  shape and at every size.
- Lanes are provisioned in whole ports, so `ports x lanes` is always **used
  plus spare**. A ports figure that forgets a remainder breaks this.
- Every port a switch gives up is counted **once**: a side's total is its
  fabric ports plus its server ports, and that total is what a port budget is
  answered against.
- The schedule and the plan are the **same fabric**: as many lines as the plan
  counted links, as many ends on a switch as it counted ports, and no lane
  plugged into twice.

The last one is what keeps `plan.rs` and `schedule.rs` honest about a
topology. They count it in completely different ways - one with arithmetic,
one by walking it - and a shape whose two answers disagree is a bug in
whichever was written second.

A `%a:b:c` ratio can be inexact for two unrelated reasons, and the report has
to say which: the ratio itself may not be cuttable from any prefix (`2:1` -
two thirds of a prefix is not a prefix), or the ratio may be fine and the
space no longer a single block. `Shares::ratio_is_dyadic` is the test that
separates them.

## What fabrictool assumes

The counting is only as good as the model, and the model is small enough to
state:

- a switch has ports of one speed, and a link runs at the link speed;
- a port carrying links slower than itself is broken out into lanes, one lane
  per link end;
- a breakout harness therefore fans one port out to several **different**
  peers, which is the whole reason it exists.

The arrangement falls out of that, and the arrangement decides the bill of
materials. Where every switch is the same - a mesh or a ring - both ends of a
link are lanes: they meet in a patch field, and the optics sit at the trunk
ports. Where they are not - a star's hub, a leaf-spine's spines - the upstream
end breaks out and everything below it gives up a whole port, which is the one
arrangement a DAC or AOC splitter can be used in, because its lanes end in
modules and a module needs a port. `Topology::is_uniform` is what that turns
on, so a new shape decides its arrangement by answering that one question.

That is why `--media=dac` with a broken-out mesh exits 3 rather than printing
a plan. It is a real constraint, not a simplification, and the error names both
ways round it.

Servers are the other half of an oversubscription ratio, and three rules keep
them honest:

- they hang off the fabric's **edge** - `Shape::edge` - which is the leaves of
  a leaf-spine, the spokes of a star, and every switch of a mesh or a ring;
- their ports are the **same lane arithmetic** as the fabric's, because a 100G
  port split four ways is four 25G servers exactly as a 400G port split four
  ways is four 100G leaves;
- the ratio is about the **servers**, not the ports they arrive on, so
  breaking out the access side changes what it costs in ports and leaves the
  ratio alone.

What is deliberately not counted is the cabling to the servers. The ports are
spent here and the ratio depends on them, but the cables and the NIC optics
are bought with the servers rather than with the fabric, so the bill of
materials stops at the leaf. What the model still does not cover is a switch
with ports at two speeds *facing the fabric*, where a splitter could fan out
into native ports in a mesh as well. Adding that means asking how many ports of
each speed a switch has, which is a question the command line does not ask.

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

`fabrictool` holds its rates in megabits per second as a `u64` for a related
reason: 2.5G is 2,500 and 1.6T is 1,600,000, so every rate anyone writes down
divides into it exactly, and a float would put rounding error into counts that
are meant to be exact. Anything finer than a megabit is refused rather than
rounded.

Switch counts are capped at 4,096 and parallel links at 64, which keeps the
products - a mesh of 4,096 is 8,386,560 links - inside a `u64` with room to
spare.

## Drawing

`--dot` is the third output mode, and the only one that shows the shape rather
than counting it. One edge per pair of switches by default; with `--schedule`,
one per cable, labelled with the ports at its ends. It is generated from the
same `Links` iterator the schedule uses, stepped over the parallel links, so a
drawing and a schedule can never disagree about what is connected to what.

DOT labels are quoted strings in which `\n` is a line break, so each line of a
node label is escaped on its own and joined afterwards - escaping the whole
label would turn the separator into the literal characters. `dot -Tsvg` over
the output is the check worth running after touching it; it is not in the test
suite because the suite may not have Graphviz.

## Shell-facing details

Operator sigils must survive an unquoted shell. `*` is a glob, which is why
`-64x2` exists alongside `-64*2`; `>` and `<` are redirection, which is why
stepping is `^N`. `@`, `^`, `%`, `+`, `=`, `/`, `.` and `:` are all safe in
bash and zsh. `~` is not, despite looking free: `~1` is directory-stack
expansion in both shells.

A new sigil has to be added to `looks_like_op` as well as to the grammar, or
it will not survive being interleaved with flags. Both tools have one of
those, and both partition argv in `arrange()` before clap sees it.

fabrictool's `xK` is the one operator with no sigil at all, so `looks_like_op`
tells it from an ordinary word by the digit after the `x`. `*K` means the same
thing and is what a shell globs, which is why both exist - as `-64x2` does on
the prefix side. `-48@25G` is told from `-n` and `--json` the same way the
prefix side does it: a digit has to follow the `-`.

**An operator carries a number; a flag carries a choice.** `@100G`, `%4`,
`x2`, `+2`, `-48@25G` and `=32@400G` all carry a figure that could be any of a
million; the shape of the fabric and what its links are made of are each one
of four, so they are `--shape` and `--media`. That line is what keeps the
grammar explicable, and a new one belongs on whichever side of it the thing
being said falls.

`looks_like_op` still claims a leading `/` and a bare `.`, which are what a
hand reaches for when the thing wanted is one of those flags: they reach
`ops::parse`, which names the flag, where clap would only say the argument was
unexpected.

`+S` carries the shape as well as a number. Saying how many spines there are
is what makes a fabric a leaf-spine, because there is no other shape the
answer fits into, and `/leaf-spine +2` says the same thing twice. Giving a
spine count to a shape that has none is an error rather than something to
quietly ignore.

`=N` is about the whole front panel, `=N@SPEED` about the ports at one speed,
and `=N@SPEED:leaf` about one kind of switch's ports at one speed. The second
exists because a real switch is specified that way - 48 at 25G, four at 100G,
two at 200G - and a single total cannot answer whether a plan fits one: four
200G uplinks fit a 54-port leaf and do not fit its two 200G ports. A speed
nothing runs at is answered as such rather than silently fitting.

The third exists because a speed picks out a kind of switch in most fabrics
but not all: leaves and spines both at 100G is an ordinary build, and without
a role the answer covers both. Covering both is the right answer to the
question as asked and still a surprising one, so the budget block says which
roles the speed reached and names the `:ROLE` that narrows it. A role the
fabric has not got is refused rather than answered yes, since it means the
question was asked of the wrong fabric.

`Role` lives in `fabric/mod.rs` alongside `Topology` and `Media` rather than
in `plan.rs`, because the grammar has to name one, and a parser reaching into
the planner for its vocabulary is the wrong way round.

A leaf-spine with no count is **a pair of spines**, not one, because that is
how a rack is built: one spine is a single point of failure rather than a
smaller fabric, and saying so is what the caution is for. The pair is also
why `spine_loss` exists - a leaf keeps its other uplink when a spine goes, at
twice the oversubscription, and that degraded ratio is the number somebody
decides two spines is enough on.

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

Both binaries ship in one archive, still named after `prefixtool`, and the
release workflow builds, signs, smoke-tests and packages every binary in
`BINS`. A third one would go in that list, in `scripts/update-formula.sh`'s
`bin.install`, and in `scripts/macos-unquarantine.sh`.

`Cargo.toml` is the only place a version lives. The flake reads it with
`fromTOML`; the Homebrew formula is generated from it.

1. Bump `version` in `Cargo.toml` on a branch, refresh `Cargo.lock`, open a PR.
2. Merging it to the default branch creates the `v<version>` tag and publishes
   the release: six targets built, then the formula regenerated and committed.

Do not create tags by hand - merging the bump is what cuts a release, and an
agent may not have permission to push tags anyway. A merge that does not change
the version is ignored, so Dependabot's manifest updates are safe.

Versions below `1.0.0`, and any with a suffix, publish as pre-releases.

## Prose

**Every sentence has a subject and a verb.** The colon is not what makes a
fragment wrong; the missing subject is. All of these need rewriting, whatever
punctuation they carry:

- "Deliberately not counted: the cabling to the servers."
- "Same twenty-eight links, but sixteen transceivers."
- "One edge per pair, labelled with what runs between them."
- "Gone. Fixed. Done."

Write "The bill of materials stops at the leaf" instead. Dropping the subject
for emphasis reads as a tic rather than as economy, and it reads that way
whether the sentence is in the README, in this file, in the help text, in a
commit message, in a pull request, or in a comment that explains something.

An imperative keeps its implied subject and is fine: "Run the suite through a
pty as well as a pipe."

The rule covers prose. It does not cover labels, which are noun phrases
because that is what a label is: the summary line of a doc comment
(`/// The shape of the fabric`), a `clap` flag description, a field value in
the report, a column heading, a table cell.

## Documentation

README examples are generated from the binary and diffed against it, not
written by hand. After changing output, regenerate the affected block and
confirm it matches:

```
diff <(./target/debug/prefixtool 2001::/64 --color=never) \
     <(sed -n '/^\$ prefixtool 2001::\/64$/,/^```$/p' README.md | sed '1d;$d')

diff <(./target/debug/fabrictool 8 @100G --color=never) \
     <(sed -n '/^\$ fabrictool 8 @100G$/,/^```$/p' README.md | sed '1d;$d')
```
