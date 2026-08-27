//! End-to-end tests: run the real binary and read its output, which is the
//! only place argument arrangement and exit codes are actually exercised.

use std::process::{Command, Output};

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_prefixtool"))
        .args(args)
        .output()
        .expect("binary runs")
}

/// Remove ANSI SGR sequences, so styled and unstyled output can be compared.
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\x1b' {
            out.push(c);
            continue;
        }
        // ESC [ ... m
        for c in chars.by_ref() {
            if c == 'm' {
                break;
            }
        }
    }
    out
}

fn stdout(args: &[&str]) -> String {
    let out = run(args);
    assert!(out.status.success(), "expected success from {args:?}");
    String::from_utf8(out.stdout).expect("utf-8 output")
}

#[test]
fn reports_on_a_bare_prefix() {
    let s = stdout(&["2001::/64"]);
    assert!(s.contains("2001::/64  -  IPv6"));
    assert!(s.contains("2^64 (~1.8e19)"));
    assert!(s.contains("Teredo"));
    assert!(s.contains("0.0.0.0.0.0.0.0.0.0.0.0.1.0.0.2.ip6.arpa."));
}

#[test]
fn the_report_never_prints_a_wall_of_digits() {
    // Anything past 2^32 is summarised, so no run of digits in the human
    // report should be longer than the readable width allows.
    for args in [
        vec!["2001:db8::/52"],
        vec!["::/0"],
        vec!["::/0", "/128"],
        vec!["2001:db8::/48", "-2001:db8:0:cc::/64"],
        vec!["2001:db8::/52", "-56", "-64x2", "/64"],
        vec!["2001:db8::/32", "+16", "/64"],
    ] {
        let s = stdout(&args);
        for line in s.lines() {
            let longest = line
                .split(|c: char| !c.is_ascii_digit() && c != ',')
                .map(|run| run.chars().filter(char::is_ascii_digit).count())
                .max()
                .unwrap_or(0);
            assert!(longest <= 13, "{args:?} printed {longest} digits: {line:?}");
        }
    }
}

#[test]
fn json_keeps_the_digits_the_report_drops() {
    // The summarising is a reading aid, not a loss of precision.
    let s = stdout(&["2001:db8::/52", "-56", "--json"]);
    assert!(s.contains("\"addresses\": 75557863725914323419136"));
    assert!(s.contains("\"free_addresses\": 70835497243044678205440"));
    let report = stdout(&["2001:db8::/52", "-56"]);
    assert!(!report.contains("75557863725914323419136"));
    assert!(report.contains("2^76 (~7.6e22)"));
}

#[test]
fn splits_a_prefix() {
    let s = stdout(&["2001:db8::/52", "/64"]);
    assert!(s.contains("Split 2001:db8::/52 into /64"));
    assert!(s.contains("Subnets        4,096"));
    assert!(s.contains("2001:db8::/64"));
    assert!(s.contains("2001:db8:0:fff::/64"));
}

#[test]
fn carves_and_aggregates_the_remainder() {
    let s = stdout(&["2001:db8::/52", "-56", "-64x2"]);
    assert!(s.contains("2001:db8::/56"));
    assert!(s.contains("2001:db8:0:100::/64"));
    assert!(s.contains("2001:db8:0:101::/64"));
    assert!(s.contains("Largest block  2001:db8:0:800::/53"));
}

#[test]
fn the_star_form_of_a_count_works_too() {
    assert_eq!(
        stdout(&["10.0.0.0/22", "-24*2", "-q"]),
        stdout(&["10.0.0.0/22", "-24x2", "-q"])
    );
}

#[test]
fn flags_and_operators_interleave() {
    let s = stdout(&["10.0.0.0/22", "/24", "-q", "-n", "2"]);
    assert_eq!(s, "10.0.0.0/24\n10.0.1.0/24\n");
}

#[test]
fn quiet_output_is_just_prefixes() {
    let s = stdout(&["10.0.0.0/22", "/24", "-q"]);
    assert_eq!(s, "10.0.0.0/24\n10.0.1.0/24\n10.0.2.0/24\n10.0.3.0/24\n");
}

#[test]
fn json_output_carries_the_exact_address_count() {
    let s = stdout(&["2001:db8::/52", "--json"]);
    assert!(s.contains("\"addresses\": 75557863725914323419136"));
    assert!(s.contains("\"reverse_dns\": \"0.0.0.0.0.8.b.d.0.1.0.0.2.ip6.arpa.\""));
}

#[test]
fn splits_into_a_number_of_subnets() {
    let s = stdout(&["10.0.0.0/24", "%5"]);
    assert!(s.contains("Split 10.0.0.0/24 into 5"));
    assert!(s.contains("3 x /26 and 2 x /27"));

    let blocks = stdout(&["10.0.0.0/24", "%5", "-q"]);
    assert_eq!(
        blocks,
        "10.0.0.0/27\n10.0.0.32/27\n10.0.0.64/26\n10.0.0.128/26\n10.0.0.192/26\n"
    );
}

#[test]
fn a_power_of_two_count_matches_the_equivalent_length() {
    assert_eq!(
        stdout(&["2001:db8::/56", "%8", "-q", "--all"]),
        stdout(&["2001:db8::/56", "/59", "-q", "--all"])
    );
}

#[test]
fn splitting_into_a_count_reaches_json() {
    let s = stdout(&["10.0.0.0/24", "%5", "--json"]);
    assert!(s.contains("\"wanted\": 5"));
    assert!(s.contains("\"source\": \"prefix\""));
    assert!(s.contains("\"prefix_length\": 26"));
    assert!(s.contains("\"10.0.0.192/26\""));
}

#[test]
fn an_impossible_count_explains_itself() {
    let out = run(&["10.0.0.0/30", "%9"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("4 is the most it holds"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn picks_a_subnet_by_number() {
    let s = stdout(&["2001:db8::/52", "/64", "@3", "@-1", "-q", "-n", "0"]);
    assert_eq!(s, "2001:db8:0:3::/64\n2001:db8:0:fff::/64\n");
}

#[test]
fn steps_along_at_the_same_size() {
    let s = stdout(&["10.0.4.0/22", "^1", "^-1", "-q"]);
    assert_eq!(s, "10.0.8.0/22\n10.0.0.0/22\n");
}

#[test]
fn aggregates_two_prefixes() {
    let s = stdout(&["10.0.0.0/24", "+10.0.1.0/24"]);
    assert!(s.contains("Aggregate 10.0.0.0/24 with 10.0.1.0/24"));
    assert!(s.contains("10.0.0.0/23"));
    assert!(s.contains("exact"));

    let s = stdout(&["10.0.0.0/24", "+10.0.3.0/24"]);
    assert!(s.contains("10.0.0.0/22"));
    assert!(s.contains("10.0.1.0/24"));
    assert!(s.contains("10.0.2.0/24"));
}

#[test]
fn several_pluses_aggregate_together_not_pairwise() {
    let s = stdout(&["10.0.0.0/24", "+10.0.1.0/24", "+10.1.0.0/16"]);

    // One aggregate covering everything, not one pairing per operator.
    assert_eq!(s.matches("Aggregate ").count(), 1, "{s}");
    assert!(s.contains("Aggregate 10.0.0.0/24 with 10.0.1.0/24 and 10.1.0.0/16"));
    assert!(s.contains("10.0.0.0/15"));

    // A prefix the user named is not spare space.
    let spare: Vec<&str> = s
        .lines()
        .skip_while(|l| !l.contains("Also covers"))
        .skip(1)
        .take_while(|l| l.starts_with("    "))
        .collect();
    assert!(
        !spare.iter().any(|l| l.contains("10.0.1.0/24")),
        "a named prefix was listed as unused: {spare:?}"
    );
    assert_eq!(spare.len(), 7);
}

#[test]
fn a_truncated_list_says_so() {
    // The aggregate's spare blocks were cut to the limit silently, hiding
    // dozens of blocks with nothing on screen to suggest they existed.
    let s = stdout(&["2001:db8::/56", "+2001:db8:ff::/64", "+2001:db8:ffff::/64"]);
    assert!(s.contains("76 blocks no input uses"));
    assert!(
        s.contains("... (showing 8 of 76; use --all or -n N)"),
        "no truncation hint: {s}"
    );

    // --all shows the lot, and then there is nothing to announce.
    let all = stdout(&[
        "2001:db8::/56",
        "+2001:db8:ff::/64",
        "+2001:db8:ffff::/64",
        "--all",
    ]);
    assert!(!all.contains("showing"), "hinted with --all: {all}");
    let listed = all
        .lines()
        .skip_while(|l| !l.contains("Also covers"))
        .skip(1)
        .take_while(|l| l.starts_with("    "))
        .count();
    assert_eq!(listed, 76, "--all did not list every block");
}

#[test]
fn a_list_exactly_the_limit_long_is_not_called_truncated() {
    // 8 subnets shown out of 8 is a complete list, not a cut-off one.
    let s = stdout(&["10.0.0.0/21", "/24", "-n", "8"]);
    assert!(s.contains("10.0.7.0/24"));
    assert!(!s.contains("showing"), "claimed truncation: {s}");

    // One more than the limit really is truncated.
    let s = stdout(&["10.0.0.0/20", "/24", "-n", "8"]);
    assert!(
        s.contains("... (showing 8 of 16; use --all or -n N)"),
        "{s}"
    );
}

#[test]
fn quiet_output_never_carries_a_hint() {
    for args in [
        vec!["10.0.0.0/20", "/24", "-q", "-n", "2"],
        vec!["2001:db8::/56", "+2001:db8:ffff::/64", "-q", "-n", "2"],
    ] {
        let s = stdout(&args);
        assert!(!s.contains("showing"), "{args:?} hinted: {s}");
        assert!(!s.contains("use --all"), "{args:?} hinted: {s}");
    }
}

#[test]
fn aggregate_inputs_reach_json_as_a_list() {
    let s = stdout(&["10.0.0.0/24", "+10.0.1.0/24", "+10.1.0.0/16", "--json"]);
    let with = s
        .lines()
        .skip_while(|l| !l.contains("\"with\""))
        .take(4)
        .collect::<Vec<_>>()
        .join("");
    assert!(with.contains("10.0.1.0/24"), "{with}");
    assert!(with.contains("10.1.0.0/16"), "{with}");
}

#[test]
fn the_new_operators_reach_json() {
    let s = stdout(&["10.0.0.0/16", "/24", "@1", "^1", "+10.1.0.0/16", "--json"]);
    assert!(s.contains("\"subnet\": \"10.0.1.0/24\""));
    assert!(s.contains("\"resolved_index\": 1"));
    assert!(s.contains("\"step\": 1"));
    assert!(s.contains("\"prefix\": \"10.1.0.0/16\""));
    assert!(s.contains("\"exact\": true"));
}

#[test]
fn a_pick_without_a_split_is_a_usage_error() {
    let out = run(&["2001:db8::/52", "@3"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("needs a split length"));
}

#[test]
fn stepping_off_the_address_space_is_a_usage_error() {
    let out = run(&["255.255.252.0/22", "^1"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("runs off"));
}

#[test]
fn the_map_shows_where_a_carve_landed() {
    let s = stdout(&["2001:db8::/56", "-2001:db8:0:cc::/64"]);
    let map: Vec<&str> = s
        .lines()
        .skip_while(|l| !l.starts_with("Map of"))
        .skip(1)
        .take_while(|l| !l.is_empty())
        .collect();
    assert_eq!(
        map,
        vec![
            "     2001:db8::/57",
            "     2001:db8:0:80::/58",
            "     2001:db8:0:c0::/61",
            "     2001:db8:0:c8::/62",
            "  -> 2001:db8:0:cc::/64   carved",
            "     2001:db8:0:cd::/64",
            "     2001:db8:0:ce::/63",
            "     2001:db8:0:d0::/60",
            "     2001:db8:0:e0::/59",
        ]
    );
}

#[test]
fn the_map_elides_long_runs_but_never_the_carve() {
    // The column width follows the widest visible row, so match the marker
    // and the prefix rather than the exact spacing between them.
    let carved = |s: &str| {
        s.lines()
            .any(|l| l.starts_with("  -> 2001:db8:0:cc::/64") && l.ends_with("carved"))
    };

    let s = stdout(&["2001:db8::/48", "-2001:db8:0:cc::/64"]);
    assert!(carved(&s), "carve missing from the map: {s}");
    assert!(s.contains("blocks, "), "no elision line: {s}");
    assert!(s.contains("(use --all)"));

    // --all shows every block, and then there is nothing left to elide.
    let all = stdout(&["2001:db8::/48", "-2001:db8:0:cc::/64", "--all"]);
    assert!(carved(&all), "carve missing under --all: {all}");
    assert!(!all.contains("(use --all)"), "elided with --all: {all}");
}

#[test]
fn the_map_covers_a_fully_allocated_parent() {
    let s = stdout(&["10.0.0.0/24", "-24"]);
    assert!(s.contains("fully allocated"));
    assert!(s.contains("  -> 10.0.0.0/24   carved"), "no map: {s}");
}

#[test]
fn the_map_is_suppressed_when_nothing_was_carved() {
    // Everything failed, so the map would only repeat the free list.
    let out = run(&["10.0.0.0/24", "-16"]);
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(!s.contains("Map of"), "pointless map: {s}");
}

#[test]
fn the_map_reaches_json() {
    let s = stdout(&["2001:db8::/56", "-2001:db8:0:cc::/64", "--json"]);
    assert!(s.contains("\"map\": ["));
    assert!(s.contains("\"prefix\": \"2001:db8:0:cc::/64\","));
    assert!(s.contains("\"carved\": true"));
    assert!(s.contains("\"carved\": false"));
}

#[test]
fn colour_never_changes_the_layout() {
    // Escape sequences have no printed width but the formatter still counts
    // them, so anything padded before styling would silently skew a column.
    // Stripping the escapes must give back exactly the uncoloured report.
    for args in [
        vec!["2001:db8::/52", "-56", "-64x2", "/64"],
        vec![
            "10.0.0.0/16",
            "-10.0.8.0/22",
            "-24x4",
            "/24",
            "+8",
            "=10.0.9.7",
        ],
        vec!["10.0.0.0/16", "/24", "@1", "@-1", "^1", "+10.1.0.0/16"],
        vec!["10.0.0.0/24", "%5"],
        vec!["10.0.0.0/22", "-24", "%5"],
        vec!["10.0.0.0/24", "%3:1"],
        vec!["10.0.0.0/24", "%2:1"],
        vec!["10.0.0.0/16", "-10.0.8.0/22", "%2:1:1"],
        vec!["10.0.0.0/8", "-30", "%2:1:1"],
        vec!["10.0.0.0/22", "."],
        vec!["10.0.0.64/26", "."],
        vec!["2001:db8::/50", "."],
        vec!["10.0.0.0/8", ".16"],
        vec!["10.0.0.0/16", "-24:dmz", "-22:wifi", "-10.0.8.0/22:legacy"],
        vec!["10.0.0.0/16", "-24:dmz", "-30"],
        vec!["10.0.0.0/16", "-24x2", "--from=top"],
        vec!["10.0.0.0/24", "-24", "-30"],
        vec!["2001:db8::/56", "-2001:db8:0:cc::/64"],
        vec!["10.0.0.0/16", "-10.0.8.0/22", "-24x3"],
        vec!["2001:db8::/48", "-2001:db8:0:cc::/64", "--all"],
        vec!["192.0.2.0/24"],
        vec!["2001::/64"],
    ] {
        let mut coloured = args.clone();
        coloured.push("--color=always");
        let mut plain = args.clone();
        plain.push("--color=never");

        let a = run(&coloured);
        let b = run(&plain);
        assert_eq!(
            strip_ansi(&String::from_utf8_lossy(&a.stdout)),
            String::from_utf8_lossy(&b.stdout),
            "layout shifted for {args:?}"
        );
    }
}

#[test]
fn colour_is_off_when_not_a_terminal() {
    // The test harness captures stdout, so auto must decide against colour.
    let s = stdout(&["192.0.2.0/24"]);
    assert!(!s.contains('\x1b'), "auto coloured a pipe");
}

#[test]
fn always_actually_colours() {
    let s = stdout(&["192.0.2.0/24", "--color=always"]);
    assert!(
        s.contains("\x1b[1;36m192.0.2.0/24\x1b[0m"),
        "prefix not styled"
    );
    // Documentation space is the one thing worth shouting about.
    assert!(s.contains("\x1b[33m"), "caution not styled");
}

#[test]
fn machine_output_is_never_coloured() {
    for extra in [vec!["--json"], vec!["-q"]] {
        let mut args = vec!["10.0.0.0/22", "/24", "--color=always"];
        args.extend(extra);
        let s = stdout(&args);
        assert!(!s.contains('\x1b'), "escapes leaked into {args:?}");
    }
}

#[test]
fn no_color_env_var_is_respected() {
    let out = Command::new(env!("CARGO_BIN_EXE_prefixtool"))
        .args(["192.0.2.0/24"])
        .env("NO_COLOR", "1")
        .output()
        .expect("binary runs");
    assert!(!String::from_utf8_lossy(&out.stdout).contains('\x1b'));
}

#[test]
fn quiet_makes_a_lookup_a_predicate() {
    // Inside is a pass, outside is a fail, the way test(1) and grep behave.
    assert_eq!(
        run(&["10.0.0.0/8", "=10.1.2.3", "-q"]).status.code(),
        Some(0)
    );
    assert_eq!(
        run(&["10.0.0.0/8", "=192.0.2.1", "-q"]).status.code(),
        Some(4)
    );

    // A prefix argument works the same as a bare address.
    assert_eq!(
        run(&["10.0.0.0/8", "=10.1.0.0/16", "-q"]).status.code(),
        Some(0)
    );
    assert_eq!(
        run(&["10.0.0.0/8", "=192.0.2.0/24", "-q"]).status.code(),
        Some(4)
    );

    // Several lookups: any one outside is a fail.
    assert_eq!(
        run(&["10.0.0.0/8", "=10.0.0.1", "=10.9.9.9", "-q"])
            .status
            .code(),
        Some(0)
    );
    assert_eq!(
        run(&["10.0.0.0/8", "=10.0.0.1", "=192.0.2.1", "-q"])
            .status
            .code(),
        Some(4)
    );
}

#[test]
fn a_mistyped_address_is_not_mistaken_for_a_no() {
    // This is why "outside" got a code of its own: a script asking whether an
    // address is inside must not read a typo as a confident answer. Bad input
    // stays at 1, outside is 4, and the two never collide.
    let out = run(&["10.0.0.0/8", "=10.1.2.999", "-q"]);
    assert_eq!(out.status.code(), Some(1));
    assert_ne!(out.status.code(), Some(4), "a typo looked like a no");
    assert!(String::from_utf8_lossy(&out.stderr).contains("not an IP prefix"));
}

#[test]
fn only_quiet_turns_a_lookup_into_an_exit_status() {
    // The report and the JSON both say so on screen, so they stay at 0.
    assert_eq!(run(&["10.0.0.0/8", "=192.0.2.1"]).status.code(), Some(0));
    assert_eq!(
        run(&["10.0.0.0/8", "=192.0.2.1", "--json"]).status.code(),
        Some(0)
    );
}

#[test]
fn an_unsatisfied_carve_outranks_a_failed_lookup() {
    // Both are wrong; the plan that could not be carried out is the bigger
    // news, so 3 wins over 4.
    let out = run(&["10.0.0.0/24", "-24", "-30", "=192.0.2.1", "-q"]);
    assert_eq!(out.status.code(), Some(3));
}

#[test]
fn an_unsatisfiable_carve_exits_three() {
    let out = run(&["10.0.0.0/24", "-24", "-30"]);
    assert_eq!(out.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&out.stdout).contains("no space left"));
}

#[test]
fn a_bad_operator_is_a_usage_error() {
    let out = run(&["10.0.0.0/24", "/16"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("did you mean +16?"));
}

#[test]
fn a_bad_prefix_is_a_usage_error() {
    let out = run(&["not-a-prefix"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("not an IP prefix"));
}

#[test]
fn listing_the_whole_ipv6_space_streams_rather_than_hangs() {
    // --all over 2^128 subnets must not try to materialise them; the -n cap
    // here is ignored, so this only returns because the writes are lazy.
    let out = Command::new(env!("CARGO_BIN_EXE_prefixtool"))
        .args(["::/0", "/128", "-q", "-n", "3"])
        .output()
        .expect("binary runs");
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "::/128\n::1/128\n::2/128\n"
    );
}

#[test]
fn a_dot_lists_the_reverse_zones_a_prefix_needs() {
    // The `Reverse DNS` line says a /22 is not a zone; `.` says which zones
    // it actually is.
    let s = stdout(&["10.0.0.0/22", "."]);
    assert!(s.contains("not on an octet boundary"));
    assert!(s.contains("Reverse zones for 10.0.0.0/22"));
    assert!(s.contains("Boundary       /24"));
    for zone in [
        "0.0.10.in-addr.arpa.",
        "1.0.10.in-addr.arpa.",
        "2.0.10.in-addr.arpa.",
        "3.0.10.in-addr.arpa.",
    ] {
        assert!(s.contains(zone), "missing {zone}");
    }
}

#[test]
fn zone_names_are_absolute() {
    // A relative name is a different name once a zone file's origin is in
    // scope, so every one of these carries the root's trailing dot - the
    // Reverse DNS line and the JSON field included, not just the new list.
    for line in stdout(&["10.0.0.0/22", "."]).lines() {
        let line = line.trim();
        if line.ends_with("arpa") {
            panic!("relative zone name: {line}");
        }
    }
    assert!(stdout(&["10.1.2.0/24"]).contains("Reverse DNS    2.1.10.in-addr.arpa."));
    assert!(stdout(&["10.1.2.0/24", "--json"]).contains("\"2.1.10.in-addr.arpa.\""));
    // The root zone is the trailing dot and nothing else in front of it.
    assert!(stdout(&["0.0.0.0/0"]).contains("Reverse DNS    in-addr.arpa."));
}

#[test]
fn a_dot_on_a_long_ipv4_prefix_explains_rfc_2317() {
    let s = stdout(&["10.0.0.64/26", "."]);
    assert!(s.contains("Parent zone    0.0.10.in-addr.arpa."));
    assert!(s.contains("64/26.0.0.10.in-addr.arpa."));
    assert!(s.contains("RFC 2317"));
}

#[test]
fn a_dot_takes_a_delegation_boundary() {
    let s = stdout(&["2001:db8::/48", ".56", "-q"]);
    let lines: Vec<&str> = s.lines().collect();
    assert_eq!(lines.len(), 8, "one per listed zone");
    assert_eq!(lines[0], "0.0.0.0.0.0.8.b.d.0.1.0.0.2.ip6.arpa.");
    assert_eq!(lines[1], "1.0.0.0.0.0.8.b.d.0.1.0.0.2.ip6.arpa.");

    // A boundary off a nibble is refused rather than rounded.
    let out = run(&["2001:db8::/48", ".50"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("delegation boundary"));
}

#[test]
fn zone_listings_stay_lazy() {
    // 2^32 zones: the first must arrive without enumerating the rest.
    let s = stdout(&["2001:db8::/32", ".64", "-q", "-n", "3"]);
    assert_eq!(s.lines().count(), 3);
    assert!(s.starts_with("0.0.0.0.0.0.0.0.8.b.d.0.1.0.0.2.ip6.arpa."));
}

#[test]
fn a_ratio_shares_the_space_out() {
    let s = stdout(&["10.0.0.0/24", "%3:1"]);
    assert!(s.contains("Share 10.0.0.0/24 in the ratio 3:1"));
    assert!(s.contains("as asked"));
    assert!(s.contains("10.0.0.0/25"));
    assert!(s.contains("10.0.0.128/26"));
    assert!(s.contains("10.0.0.192/26"));
}

#[test]
fn a_ratio_that_cannot_be_cut_exactly_says_what_it_gave_instead() {
    // Two thirds of a prefix is not a prefix, so 2:1 lands on 3:1 - the same
    // bargain %3 makes when it hands out 2:1:1 for three equal parties.
    let s = stdout(&["10.0.0.0/24", "%2:1"]);
    assert!(s.contains("3:1  for a request of 2:1"));
    assert!(s.contains("power of two"));
}

#[test]
fn a_ratio_of_all_ones_matches_the_same_count_of_parts() {
    let sizes = |args: &[&str]| {
        let mut v: Vec<String> = stdout(args)
            .lines()
            .map(|l| l.split('/').nth(1).unwrap_or_default().to_string())
            .collect();
        v.sort();
        v
    };
    assert_eq!(
        sizes(&["10.0.0.0/24", "%1:1:1", "-q"]),
        sizes(&["10.0.0.0/24", "%3", "-q"])
    );
    assert_eq!(
        sizes(&["10.0.0.0/24", "%1:1:1:1:1", "-q"]),
        sizes(&["10.0.0.0/24", "%5", "-q"])
    );
}

#[test]
fn a_ratio_over_a_ragged_remainder_reads_as_percentages() {
    // The unit that divides every piece of a carved-up /8 is fine enough that
    // the achieved ratio has ten digits, which tells nobody anything.
    let s = stdout(&["10.0.0.0/8", "-30", "%2:1:1"]);
    assert!(s.contains("50.0% : 25.0% : 25.0%  for a request of 2:1:1"));
    assert!(s.contains("it is no longer one block"));
}

#[test]
fn a_ratio_the_space_cannot_hold_is_a_usage_error() {
    let out = run(&["10.0.0.0/30", "%2:1:1:1"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("cannot be shared"));
}

#[test]
fn a_named_carve_carries_its_name_into_the_table_and_the_map() {
    let s = stdout(&["10.0.0.0/16", "-24:dmz", "-22:wifi", "-10.0.8.0/22:legacy"]);
    assert!(s.contains("Name"));
    assert!(s.contains("legacy"));
    // The map says what each allocation is, rather than just "carved".
    assert!(s.contains("-> 10.0.8.0/22    legacy"));
    assert!(s.contains("wifi"));
    assert!(s.contains("dmz"));
    assert!(
        !s.contains("carved"),
        "a named map row should not say carved"
    );
}

#[test]
fn an_unnamed_carve_looks_exactly_as_it_did() {
    let s = stdout(&["10.0.0.0/16", "-24"]);
    assert!(!s.contains("Name"), "no name column without a name");
    assert!(s.contains("carved"));
}

#[test]
fn a_name_survives_a_prefix_full_of_colons() {
    let s = stdout(&["2001:db8::/48", "-2001:db8:0:cc::/64:core", "-q"]);
    assert_eq!(s.lines().next(), Some("2001:db8:0:cc::/64"));
    let s = stdout(&["2001:db8::/48", "-2001:db8:0:cc::/64:core"]);
    assert!(s.contains("core"));
}

#[test]
fn from_top_fills_the_other_end() {
    let bottom = stdout(&["10.0.0.0/16", "-24x2", "-q"]);
    let top = stdout(&["10.0.0.0/16", "-24x2", "--from=top", "-q"]);
    assert!(bottom.starts_with("10.0.0.0/24\n10.0.1.0/24\n"));
    assert!(top.starts_with("10.0.255.0/24\n10.0.254.0/24\n"));
    assert!(stdout(&["10.0.0.0/16", "-24", "--from=top"]).contains("filling from the top"));
}

#[test]
fn from_top_may_be_given_among_the_operators() {
    // arrange() has to keep --from out of the operator list like any flag.
    let s = stdout(&["10.0.0.0/16", "-24", "--from", "top", "-q"]);
    assert_eq!(s.lines().next(), Some("10.0.255.0/24"));
}

#[test]
fn the_newest_operators_reach_json() {
    let s = stdout(&[
        "10.0.0.0/22",
        ".",
        "%2:1:1",
        "-24:dmz",
        "--from=top",
        "--json",
    ]);
    for key in [
        "\"reverse_zones\"",
        "\"kind\": \"aligned\"",
        "\"boundary\": 24",
        "\"shares\"",
        "\"achieved\"",
        "\"exact\"",
        "\"direction\": \"top\"",
        "\"name\": \"dmz\"",
    ] {
        assert!(s.contains(key), "missing {key} from --json");
    }
    // Classless delegations get their own shape.
    let s = stdout(&["10.0.0.64/26", ".", "--json"]);
    assert!(s.contains("\"kind\": \"classless\""));
    assert!(s.contains("\"rfc\": \"RFC 2317\""));
}
