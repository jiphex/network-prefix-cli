//! End-to-end tests for fabrictool: run the real binary and read its output,
//! which is the only place argument arrangement and exit codes are actually
//! exercised.

use std::process::{Command, Output};

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_fabrictool"))
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

fn stderr(args: &[&str]) -> String {
    String::from_utf8(run(args).stderr).expect("utf-8 output")
}

#[test]
fn reports_on_a_bare_switch_count() {
    let s = stdout(&["8 switches -- mesh"]);
    assert!(s.starts_with("8 switches  -  full mesh"), "{s}");
    assert!(s.contains("Links          28"), "{s}");
    assert!(s.contains("7 ports on each switch"), "{s}");
    // Nothing was said about speed, so nothing is claimed about bandwidth.
    assert!(!s.contains("Link speed"), "{s}");
}

#[test]
fn counts_the_optics_a_breakout_saves() {
    let s = stdout(&["8 switches[400G] -100G- mesh"]);
    assert!(s.contains("2 x 400G on each switch"), "{s}");
    assert!(s.contains("16  400G transceiver"), "{s}");
    assert!(s.contains("28  duplex coupler"), "{s}");
    // The straight version of the same fabric buys more than three times as
    // many transceivers, which is the entire point of the comparison.
    let straight = stdout(&["8 switches -100G- mesh"]);
    assert!(straight.contains("56  100G transceiver"), "{straight}");
}

#[test]
fn the_shape_changes_the_counts() {
    assert!(stdout(&["16 switches -- ring"]).contains("Links          16"));
    assert!(stdout(&["1 hub -- 15 spokes"]).contains("Links          15"));
    assert!(stdout(&["16 switches -- mesh"]).contains("Links          120"));
    assert!(stdout(&["16 switches -2x- mesh"]).contains("Links          240"));
}

#[test]
fn flags_and_operators_may_be_interleaved() {
    let a = stdout(&["8 switches -100G- mesh", "--json"]);
    let b = stdout(&["8 switches -100G- mesh", "--json"]);
    let c = stdout(&["8 switches -100G- mesh", "--json"]);
    assert_eq!(a, b);
    assert_eq!(a, c);
    // -n takes a number, and that number is not the switch count.
    let s = stdout(&[
        "8 switches -- mesh",
        "-n",
        "2",
        "--schedule",
        "--color=never",
    ]);
    assert!(s.contains("... (showing 2 of 28"), "{s}");
}

#[test]
fn a_schedule_lists_every_link_when_asked() {
    let s = stdout(&["8 switches -- mesh", "--schedule", "--color=never"]);
    let patches = s.lines().filter(|l| l.contains("->")).count();
    assert_eq!(patches, 28);
    assert!(s.contains("sw1:1  ->  sw2:1"), "{s}");
    assert!(!s.contains("showing"), "{s}");
}

#[test]
fn quiet_is_a_bill_of_materials_one_item_per_line() {
    let s = stdout(&["8 switches[400G] -100G- mesh", "-q"]);
    assert_eq!(
        s,
        "16\ttransceiver-400G\n16\tbreakout-1x4-400G\n28\tcoupler-100G\n16\tport-switch-400G\n"
    );
    // And the schedule when the schedule is what was asked for.
    let s = stdout(&["4 switches -- mesh", "--schedule", "-q"]);
    assert_eq!(s.lines().count(), 6);
    assert_eq!(s.lines().next(), Some("sw1:1 sw2:1"));
}

#[test]
fn machine_output_is_never_coloured() {
    for args in [
        vec!["8 switches -100G- mesh", "-q", "--color=always"],
        vec!["8 switches -100G- mesh", "--json", "--color=always"],
    ] {
        let s = stdout(&args);
        assert!(!s.contains('\x1b'), "{args:?} came out coloured");
    }
}

#[test]
fn colour_never_changes_the_layout() {
    for args in [
        vec!["8 switches[400G,32] -100G- mesh", "--schedule"],
        vec!["1 hub[100G,8] -25G- 8 spokes[8]"],
        vec!["4 switches -2x- ring"],
    ] {
        let mut painted = args.clone();
        painted.push("--color=always");
        let mut plain = args.clone();
        plain.push("--color=never");
        assert_eq!(strip_ansi(&stdout(&painted)), stdout(&plain), "{args:?}");
    }
}

#[test]
fn json_is_exact_and_parseable_shaped() {
    let s = stdout(&["8 switches[400G,32] -100G- mesh", "--schedule", "--json"]);
    for want in [
        "\"switches\": 8",
        "\"links\": 28",
        "\"link_speed_mbps\": 100000",
        "\"port_speed_mbps\": 400000",
        "\"arrangement\": \"split-both-ends\"",
        "\"trunk_ports\": 16",
        "\"total_mbps\": 2800000",
        "\"fits\": true",
    ] {
        assert!(s.contains(want), "{want} missing from {s}");
    }
}

#[test]
fn a_port_budget_is_answered_in_the_report() {
    let s = stdout(&["8 switches[400G,32] -100G- mesh", "--color=never"]);
    assert!(
        s.contains("Ports on a 32-port switch\n  yes - it fits"),
        "{s}"
    );
    let out = run(&["1 hub[32] -100G- 47 spokes[32]", "--color=never"]);
    let s = String::from_utf8(out.stdout).unwrap();
    assert!(s.contains("no - the hub is 15 short"), "{s}");
    // Only --quiet turns the question into an exit status; on screen the
    // answer is there to be read.
    assert_eq!(out.status.code(), Some(0));
}

#[test]
fn under_quiet_a_port_budget_is_the_exit_status() {
    assert_eq!(
        run(&["8 switches[400G,32] -100G- mesh", "-q"])
            .status
            .code(),
        Some(0)
    );
    assert_eq!(
        run(&["1 hub[32] -- 47 spokes[32]", "-q"]).status.code(),
        Some(4)
    );
    // A typo is not a confident no.
    let out = run(&["1 hub[lots] -- 47 spokes[lots]", "-q"]);
    assert_eq!(out.status.code(), Some(1));
    assert_ne!(out.status.code(), Some(4), "a typo looked like a no");
}

#[test]
fn a_leaf_spine_counts_its_two_populations_apart() {
    let s = stdout(&[
        "2 spines[400G] -100G- 16 leaves -25G- 48 servers",
        "--color=never",
    ]);
    assert!(
        s.starts_with("16 leaves + 2 spines  -  leaf-spine at 100G"),
        "{s}"
    );
    assert!(s.contains("Links          32"), "{s}");
    assert!(s.contains("4 x 400G on each spine"), "{s}");
    assert!(s.contains("2 x 100G on each leaf"), "{s}");
    assert!(s.contains("Server ports   48 x 25G on each leaf"), "{s}");
    assert!(s.contains("Per leaf       6:1"), "{s}");
    // A 400G spine port reaches four leaves, so four ports carry sixteen.
    assert!(s.contains("8  400G transceiver"), "{s}");
    assert!(s.contains("32  100G transceiver"), "{s}");
}

/// The arrangement a rack is actually built in: a pair of leaves under a
/// pair of spines, every leaf on every spine.
#[test]
fn a_rack_is_a_pair_of_spines_and_survives_losing_one() {
    let s = stdout(&["2 spines -100G- 2 leaves -25G- 48 servers", "--color=never"]);
    assert!(
        s.starts_with("2 leaves + 2 spines  -  leaf-spine at 100G"),
        "{s}"
    );
    assert!(s.contains("Links          4"), "{s}");
    assert!(s.contains("each leaf 2 links, 200G"), "{s}");
    assert!(
        s.contains("Spine loss     each leaf keeps 1 uplink of 2"),
        "{s}"
    );
    assert!(s.contains("One spine down 12:1"), "{s}");
    // Both leaves reach both spines, and nothing is wired twice.
    let schedule = stdout(&["2 spines -- 2 leaves", "--schedule", "-q"]);
    assert_eq!(
        schedule,
        "spine1:1 leaf1:1\nspine1:2 leaf2:1\nspine2:1 leaf1:2\nspine2:2 leaf2:2\n"
    );
}

#[test]
fn a_leaf_spine_is_a_pair_of_spines_unless_told_otherwise() {
    let s = stdout(&["2 spines -100G- 16 leaves", "--color=never"]);
    assert!(s.starts_with("16 leaves + 2 spines"), "{s}");
    // One is still allowed, and still says what it is.
    let s = stdout(&["1 spines -100G- 16 leaves", "--color=never"]);
    assert!(s.starts_with("16 leaves + 1 spine"), "{s}");
    assert!(s.contains("single point of failure"), "{s}");
}

#[test]
fn uplinks_change_the_ratio_and_nothing_else_about_the_servers() {
    let ratio = |spines: u32| {
        let s = stdout(&[
            &format!("{spines} spines -100G- 16 leaves -25G- 48 servers"),
            "--color=never",
        ]);
        s.lines()
            .find(|l| l.trim_start().starts_with("Per leaf"))
            .map(|l| l.split_whitespace().nth(2).unwrap().to_string())
            .expect("a ratio")
    };
    assert_eq!(ratio(2), "6:1");
    assert_eq!(ratio(4), "3:1");
    assert_eq!(ratio(6), "2:1");
    assert_eq!(ratio(12), "1:1");
}

#[test]
fn server_ports_count_against_the_port_budget() {
    // Two uplinks and 48 servers is 50 ports, whatever they are facing.
    let s = stdout(&[
        "2 spines -100G- 16 leaves[56] -25G- 48 servers",
        "--color=never",
    ]);
    assert!(
        s.contains("each leaf  50 of 56 ports (2 at 100G, 48 at 25G)"),
        "{s}"
    );
    assert_eq!(
        run(&["2 spines -100G- 16 leaves[48] -25G- 48 servers", "-q"])
            .status
            .code(),
        Some(4),
        "50 ports do not fit in 48"
    );
    // Arriving on split ports, the same servers take twelve ports, not 48.
    let s = stdout(&[
        "2 spines -100G- 16 leaves[100G,56] -25G- 48 servers",
        "--color=never",
    ]);
    assert!(s.contains("each leaf  14 of 56 ports"), "{s}");
}

/// Four 32-port 400G spines, four racks of paired leaves, and leaves whose
/// front panel is 48x25G + 2x200G + 4x100G: which cabling options fit.
#[test]
fn a_real_build_is_checked_against_the_leaf_it_has() {
    // The leaf's real front panel, written once on the tier that has it.
    const LEAF: &str = "[48x25G,2x200G,4x100G]";

    // 100G to every spine, out of the spine's 400G ports split four ways.
    let s = stdout(&[
        &format!("4 spines[32x400G] -100G- 8 leaves{LEAF} -25G- 48 servers"),
        "--color=never",
    ]);
    assert!(
        s.contains("each spine  2 of 32 ports at 400G, 30 spare"),
        "{s}"
    );
    assert!(
        s.contains("each leaf  4 of 4 ports at 100G, 0 spare"),
        "{s}"
    );
    assert!(
        s.contains("each leaf  48 of 48 ports at 25G, 0 spare"),
        "{s}"
    );
    assert!(!s.contains(" no - "), "{s}");

    // 200G to every spine needs four 200G ports on a leaf that has two, so
    // the full mesh cannot be kept at that speed.
    let s = stdout(&[
        &format!("4 spines[32x400G] -200G- 8 leaves{LEAF} -25G- 48 servers"),
        "--color=never",
    ]);
    assert!(
        s.contains("200G ports on a 2-port leaf\n  no - each leaf is 2 short"),
        "{s}"
    );
    assert_eq!(
        run(&["4 spines[400G] -200G- 8 leaves[2x200G]", "-q"])
            .status
            .code(),
        Some(4)
    );
}

/// A speed picks out a kind of switch in most fabrics; where it does not,
/// the question has to be able to say which switches it is about.
#[test]
fn a_budget_can_name_the_switches_it_is_about() {
    // Leaves and spines both at 100G here, so the speed alone cannot separate
    // them: the leaves fit four ports and the spines do not.
    let s = stdout(&["2 spines -100G- 8 leaves[4x100G]", "--color=never"]);
    assert!(s.contains("100G ports on a 4-port leaf"), "{s}");
    assert!(
        s.contains("each leaf  2 of 4 ports at 100G, 2 spare"),
        "{s}"
    );
    // The report still describes both kinds of switch; it is the answer to
    // the question that is about leaves alone.
    let answer = s
        .split("100G ports on a 4-port leaf")
        .nth(1)
        .expect("the budget section");
    assert!(!answer.contains("each spine"), "{s}");

    let out = run(&["2 spines[4x100G] -100G- 8 leaves", "--color=never"]);
    let s = String::from_utf8(out.stdout).unwrap();
    assert!(s.contains("100G ports on a 4-port spine"), "{s}");
    assert!(s.contains("no - each spine is 4 short"), "{s}");
    assert_eq!(
        run(&["2 spines[4x100G] -100G- 8 leaves", "-q"])
            .status
            .code(),
        Some(4)
    );

    // A tier the shape has not got is a misunderstanding of the fabric
    // rather than a plan that does not fit, so the notation refuses it.
    let out = run(&["8 leaves -100G- mesh"]);
    assert_eq!(out.status.code(), Some(1));
    let e = String::from_utf8(out.stderr).unwrap();
    assert!(e.contains("one tier of switches"), "{e}");
    assert!(stderr(&["8 racks -100G- mesh"]).contains("not a kind of tier"));

    let out = run(&["8 switches[400G] -100G:dac- mesh"]);
    assert_eq!(out.status.code(), Some(3));
    let e = String::from_utf8(out.stderr).unwrap();
    assert!(e.contains("a hub over spokes"), "{e}");

    // Bad input, by contrast, is 1 all the way down.
    for fabric in [
        "1 switches -- mesh",
        "0 switches -- mesh",
        "banana",
        "8 switches -banana- mesh",
        "8 switches -- banana",
        "8 switches[] -100G- mesh",
        "8 switches[40G] -100G- mesh",
        "8 switches -100G- -400G- mesh",
        "2 spines -- 8 switches",
        "0 spines -- 8 leaves",
        "2 spines -100G- 8 leaves -25G- 48",
        "2 spines -100G- 8 leaves -banana- 48 servers",
        "2 spines -400G- 8 spines -100G- 16 leaves",
    ] {
        assert_eq!(run(&[fabric]).status.code(), Some(1), "{fabric}");
    }
}

#[test]
fn errors_say_what_to_do_about_it() {
    assert!(stderr(&["1 switches -- mesh"]).contains("not a fabric"));
    // A tier says what it counts, and the message lists what it could be.
    assert!(stderr(&["8 -- mesh"]).contains("spines, leaves, switches"));
    assert!(stderr(&["8 racks -- mesh"]).contains("not a kind of tier"));
    // A port that cannot carry its link says what the switch does have.
    assert!(stderr(&["8 switches[100G] -400G- mesh"]).contains("no port on"));
    // A third tier of switches is named for what it is.
    assert!(stderr(&["2 spines -400G- 8 spines -100G- 16 leaves"]).contains("super-spine layer"));
    // On stderr, prefixed, so it never reaches a pipeline's data - and a
    // run that worked says nothing there at all.
    assert!(stderr(&["1 switches -- mesh"]).starts_with("fabrictool:"));
    assert!(stderr(&["8 switches -- mesh"]).is_empty());
}

#[test]
fn the_report_never_argues_with_itself() {
    // Every count in the report is derived from the same plan, so the
    // headline link count and the schedule have to agree, whatever the shape.
    for args in [
        vec!["7 switches -- mesh", "--schedule"],
        vec!["7 switches -- ring", "--schedule"],
        vec!["1 hub -- 6 spokes", "--schedule"],
        vec!["5 switches -3x- mesh", "--schedule"],
        vec!["6 switches[400G] -100G- mesh", "--schedule"],
        vec!["2 spines -- 6 leaves", "--schedule"],
        vec![
            "3 spines[400G] -100G- 6 leaves[24x10G] -10G- 24 servers",
            "--schedule",
        ],
    ] {
        let mut with_colour = args.clone();
        with_colour.push("--color=never");
        let s = stdout(&with_colour);
        let listed = s.lines().filter(|l| l.contains("->")).count();
        let headline = s
            .lines()
            .find(|l| l.trim_start().starts_with("Links "))
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|n| n.replace(',', "").parse::<usize>().ok())
            .expect("a link count");
        assert_eq!(listed, headline, "{args:?}");
    }
}
