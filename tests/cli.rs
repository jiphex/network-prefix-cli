//! End-to-end tests: run the real binary and read its output, which is the
//! only place argument arrangement and exit codes are actually exercised.

use std::process::{Command, Output};

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_prefixtool"))
        .args(args)
        .output()
        .expect("binary runs")
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
    assert!(s.contains("18,446,744,073,709,551,616"));
    assert!(s.contains("Teredo"));
    assert!(s.contains("0.0.0.0.0.0.0.0.0.0.0.0.1.0.0.2.ip6.arpa"));
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
    assert!(s.contains("siblings"));

    let s = stdout(&["10.0.0.0/24", "+10.0.3.0/24"]);
    assert!(s.contains("10.0.0.0/22"));
    assert!(s.contains("10.0.1.0/24"));
    assert!(s.contains("10.0.2.0/24"));
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
