use super::*;
use crate::scratch::Scratch;
use serde_json::json;

fn spent(cost_micros: u64, input: u64, output: u64) -> Usage {
    Usage {
        input,
        output,
        cost_micros,
        ..Usage::default()
    }
}

/// The cap a config chunk leaves, read the way an ask reads it.
fn configured(source: &str) -> Option<u64> {
    let mut engine = crate::mind::lua::engine::Engine::new();
    engine.run(source, "test").expect("runs");
    engine.harvest();
    cap(engine.config().get(SETTING))
}

#[test]
fn unset_is_ten_dollars_and_zero_or_false_is_no_cap() {
    assert_eq!(cap(None), Some(10_000_000));
    assert_eq!(configured(""), Some(10_000_000), "the default when unset");
    for off in ["0", "0.0", "false"] {
        assert_eq!(
            configured(&format!("melchior.spend_cap_usd = {off}")),
            None,
            "{off}"
        );
    }
    assert_eq!(configured("melchior.spend_cap_usd = 2.5"), Some(2_500_000));
    for odd in [json!(-3), json!("lots"), json!(true), json!({})] {
        assert_eq!(cap(Some(&odd)), Some(10_000_000), "{odd} kept the default");
    }
}

#[test]
fn the_setting_is_declared_and_a_coordinator_may_set_it() {
    let declared = crate::mind::setup::needs();
    let need = declared
        .iter()
        .find(|need| need.name == SETTING)
        .expect("declared");
    assert_eq!(need.default, Some(json!(10.0)));
    assert!(need.about.contains("0 or false"), "{need:?}");

    let path = Scratch::file("melchior-spend", "configure", "given.lua");
    let applied = crate::mind::setup::configure_into(Some(&path), "melchior.spend_cap_usd = 25")
        .expect("runs");
    assert_eq!(applied.set, vec![SETTING.to_owned()], "{applied:?}");
}

#[test]
fn a_run_at_its_cap_is_refused_and_told_how_to_go_on() {
    let said = refused(2_000_000, Some(1_000_000)).expect("over");
    for part in ["$2.00", "$1.00", "set melchior.spend_cap_usd"] {
        assert!(said.contains(part), "{part} missing: {said}");
    }
    assert!(refused(1_000_000, Some(1_000_000)).is_some(), "at the cap");
    assert_eq!(refused(999_999, Some(1_000_000)), None);
    assert_eq!(refused(u64::MAX, None), None, "no cap");
    assert_eq!(dollars(10_023_400), "$10.0234");
    assert_eq!(dollars(5_000), "$0.005");
    assert_eq!(dollars(10_500_000), "$10.50");
}

#[test]
fn what_an_ask_cost_is_what_the_provider_said_else_the_model_s_rates() {
    let rates = Cost {
        input: 3.0,
        output: 15.0,
        ..Cost::default()
    };
    assert_eq!(
        cost(spent(150, 1_000, 1_000), rates),
        150,
        "the provider's word"
    );
    assert_eq!(cost(spent(0, 1_000_000, 100_000), rates), 4_500_000);
    assert_eq!(cost(spent(0, 1_000_000, 100_000), Cost::default()), 0);
}

#[test]
fn each_spent_adds_what_is_new_and_a_failed_attempt_stays_paid() {
    let dir = Scratch::new("melchior-spend", "meter");
    let ledger = Ledger::at(&dir, "alpha-rho-1");
    let mut meter = Meter::new(Some(ledger.clone()), Cost::default());
    // An attempt that said what it cost, failed, and was said again as paid before the next.
    for heard in [100, 100, 100, 250, 250] {
        meter.heard(spent(heard, 1, 1));
    }
    assert_eq!(ledger.spent(), 250);
    let mut another = Meter::new(Some(ledger.clone()), Cost::default());
    another.heard(spent(40, 1, 1));
    assert_eq!(
        ledger.spent(),
        290,
        "a second ask in the run adds to the first"
    );
}

#[test]
fn a_model_with_no_price_costs_nothing_and_writes_nothing() {
    let dir = Scratch::new("melchior-spend", "free");
    let ledger = Ledger::at(&dir.join("spent"), "alpha-rho-1");
    Meter::new(Some(ledger.clone()), Cost::default()).heard(spent(0, 5_000, 900));
    assert_eq!(ledger.spent(), 0);
    assert!(!dir.join("spent").exists(), "a free ask made a ledger");
}

#[test]
fn asks_adding_at_once_lose_nothing() {
    use std::os::unix::fs::MetadataExt;
    let dir = Scratch::new("melchior-spend", "racing");
    let kept = dir.join("spent");
    let threads: Vec<_> = (0..8)
        .map(|_| {
            let ledger = Ledger::at(&kept, "alpha-rho-1");
            std::thread::spawn(move || {
                for _ in 0..25 {
                    ledger.add(7).expect("added");
                }
            })
        })
        .collect();
    for thread in threads {
        thread.join().expect("joined");
    }
    let ledger = Ledger::at(&kept, "alpha-rho-1");
    assert_eq!(ledger.spent(), 8 * 25 * 7);
    let mode = |path: &Path| std::fs::metadata(path).expect("there").mode() & 0o777;
    assert_eq!(mode(&kept), 0o700);
    assert_eq!(mode(&kept.join("alpha-rho-1")), 0o600);

    ledger.close();
    assert!(!kept.exists(), "closing the last ledger left its directory");
}
