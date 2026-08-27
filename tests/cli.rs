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
    assert!(s.contains("0.0.0.0.0.0.0.0.0.0.0.0.1.0.0.2.ip6.arpa"));
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
    assert!(s.contains("\"reverse_dns\": \"0.0.0.0.0.8.b.d.0.1.0.0.2.ip6.arpa\""));
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
fn a_pick_without_a_split_exits_one_with_a_hint() {
    let out = run(&["2001:db8::/52", "@3"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("needs a split length"));
}

#[test]
fn stepping_off_the_address_space_exits_one() {
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
fn an_unsatisfiable_carve_exits_three() {
    let out = run(&["10.0.0.0/24", "-24", "-30"]);
    assert_eq!(out.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&out.stdout).contains("no space left"));
}

#[test]
fn a_bad_operator_exits_one_with_a_hint() {
    let out = run(&["10.0.0.0/24", "/16"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("did you mean +16?"));
}

#[test]
fn a_bad_prefix_exits_one() {
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
