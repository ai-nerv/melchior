#![allow(clippy::expect_used, clippy::indexing_slicing)]
use super::*;
use serde_json::{Value, json};
use std::{
    collections::HashSet,
    sync::{Arc, Barrier},
    time::{Duration, Instant},
};

const GOLDEN: &str = include_str!("golden.json");

#[test]
fn receiver_shutdown_counts_queued_but_not_dequeued_events() {
    let (producer, receiver) = pair();
    for _ in 0..3 {
        assert_eq!(producer.emit(prepared), Emitted::Queued);
    }
    receiver.try_recv().expect("one event handed to consumer");
    assert_eq!(producer.loss.snapshot().abandoned, 0);
    drop(receiver);
    assert_eq!(producer.loss.snapshot().abandoned, 2);
    assert_eq!(producer.emit(prepared), Emitted::Closed);
    assert_eq!(producer.loss.snapshot().closed, 1);
    assert_eq!(producer.loss.snapshot().abandoned, 2);
}

#[test]
fn receiver_shutdown_racing_production_accounts_for_every_attempt() {
    let (producer, receiver) = pair();
    let producer = Arc::new(producer);
    let barrier = Arc::new(Barrier::new(2));
    let task = std::thread::spawn({
        let (producer, barrier) = (producer.clone(), barrier.clone());
        move || {
            barrier.wait();
            for _ in 0..1000 {
                assert!(matches!(
                    producer.emit(prepared),
                    Emitted::Queued | Emitted::Full | Emitted::Closed
                ));
            }
        }
    });
    barrier.wait();
    drop(receiver);
    task.join().expect("producer settled after shutdown");
    let loss = producer.loss.snapshot();
    assert_eq!(loss.abandoned + loss.full + loss.closed, 1000);
    assert_eq!(loss.encoding + loss.exhausted, 0);
}

#[test]
fn usage_snapshot_revisions_cost_bases_and_ledger_status_are_separate() {
    let (producer, receiver) = pair();
    let usage = Fact::UsageSnapshot {
        revision: NonZeroU64::new(2).expect("snapshot revision"),
        input_tokens: Some(0),
        output_tokens: None,
        cache_read_tokens: Some(10),
        cache_write_tokens: None,
        reported_cost_micros: Some(7),
        estimated_cost_micros: Some(3),
        ledger: LedgerOutcome::Failed,
    };
    assert_eq!(
        producer.emit(|| (
            Correlation {
                ask_id: Some(identity()),
                ..Correlation::default()
            },
            usage
        )),
        Emitted::Queued
    );
    let bytes = receiver.try_recv().expect("snapshot");
    let event = decode(&bytes).expect("typed snapshot");
    assert_eq!(event.fact, usage);
    assert_eq!(event.correlation.ask_id, Some(identity()));
    let value: Value = serde_json::from_slice(&bytes).expect("independent JSON");
    assert_eq!(value["fact"]["input_tokens"], 0);
    assert!(value["fact"]["output_tokens"].is_null());
    assert_eq!(value["fact"]["ledger"], "failed");
    for (field, replacement) in [
        ("revision", json!(0)),
        ("revision", json!(-1)),
        ("reported_cost_micros", json!(-1)),
        ("estimated_cost_micros", json!(1.5)),
        ("output_tokens", json!("unknown")),
        ("ledger", json!("secret-error-canary")),
        ("request_body", json!("secret-content-canary")),
    ] {
        let mut changed = value.clone();
        changed["fact"][field] = replacement;
        assert_eq!(
            decode(&serde_json::to_vec(&changed).expect("JSON")),
            Err(DecodeError::Invalid)
        );
    }
}

fn identity() -> Id {
    "00112233445566778899aabbccddeeff"
        .to_owned()
        .try_into()
        .expect("opaque ID")
}
fn prepared() -> (Correlation, Fact) {
    (
        Correlation::default(),
        Fact::ProviderAttempt {
            outcome: AttemptOutcome::Cancelled,
            duration_ms: None,
        },
    )
}
fn pair() -> (Emitter, Receiver) {
    Emitter::new(identity(), [0, 7, 3])
}

#[test]
fn golden_is_exact_and_missing_measurements_remain_null() {
    let (producer, receiver) = pair();
    assert_eq!(producer.emit(prepared), Emitted::Queued);
    let bytes = receiver.try_recv().expect("queued event");
    assert_eq!(bytes, GOLDEN.trim_end().as_bytes());
    let event = decode(&bytes).expect("independent golden schema");
    assert_eq!(event.producer, identity());
    assert_eq!(event.sequence.get(), 1);
    assert_eq!(event.correlation, Correlation::default());
    assert_eq!(event.fact, prepared().1);
}

#[test]
fn revision_owner_kinds_fields_and_numeric_states_are_strict() {
    let original: Value = serde_json::from_str(GOLDEN).expect("golden");
    for (pointer, replacement) in [
        ("/schema", json!(2)),
        ("/schema", json!(-1)),
        ("/schema", json!("1")),
        ("/program", json!("magi")),
        ("/sequence", json!(0)),
        ("/sequence", json!(-1)),
        ("/sequence", json!(1.5)),
        ("/program_version", json!([0, 7, 65536])),
        ("/fact/kind", json!("provider_attempt_ended")),
        ("/fact/outcome", json!("secret-error-canary")),
        ("/fact/duration_ms", json!(-1)),
        ("/fact/duration_ms", json!(1.5)),
        ("/correlation/run_id", json!("/private/secret-path")),
    ] {
        let mut changed = original.clone();
        *changed.pointer_mut(pointer).expect("known pointer") = replacement;
        let error = decode(&serde_json::to_vec(&changed).expect("JSON")).expect_err(pointer);
        assert_eq!(error, DecodeError::Invalid);
        assert!(!format!("{error:?}").contains("secret"));
    }
    for pointer in ["", "/fact", "/correlation"] {
        let mut changed = original.clone();
        changed
            .pointer_mut(pointer)
            .expect("object")
            .as_object_mut()
            .expect("map")
            .insert("private-content".into(), json!("secret-header-canary"));
        assert_eq!(
            decode(&serde_json::to_vec(&changed).expect("JSON")),
            Err(DecodeError::Invalid)
        );
    }
    let duplicate = GOLDEN.replacen("\"schema\":1", "\"schema\":1,\"schema\":1", 1);
    assert_eq!(decode(duplicate.as_bytes()), Err(DecodeError::Invalid));
}

#[test]
fn opaque_identifiers_reject_hostile_and_noncanonical_labels() {
    for value in [
        "",
        "abcd",
        "00112233445566778899AABBCCDDEEFF",
        "../private/path",
        "Bearer secret-token",
        "éééééééééééééééé",
        "00112233445566778899aabbccddeeff\n",
    ] {
        assert!(Id::try_from(value.to_owned()).is_err());
    }
    let first = Id::generate().expect("OS nonce");
    let second = Id::generate().expect("new producer nonce");
    assert_ne!(first, second);
    assert_eq!(
        Id::try_from(String::from(first)).expect("canonical roundtrip"),
        first
    );
}

#[test]
fn encoded_and_decoded_limits_are_enforced_before_unbounded_growth() {
    let mut encoder = Encoded {
        bytes: [0; MAX_EVENT_BYTES],
        len: 0,
    };
    encoder
        .write_all(&[b'x'; MAX_EVENT_BYTES])
        .expect("exact limit");
    assert!(encoder.write_all(b"x").is_err());
    assert_eq!(encoder.len, MAX_EVENT_BYTES);
    let mut bytes = GOLDEN.trim_end().as_bytes().to_vec();
    bytes.resize(MAX_EVENT_BYTES, b' ');
    assert!(decode(&bytes).is_ok());
    bytes.push(b' ');
    assert_eq!(decode(&bytes), Err(DecodeError::Oversized));
}

#[test]
fn full_queue_drops_without_waiting_for_a_stalled_consumer() {
    let (producer, receiver) = pair();
    let producer = Arc::new(producer);
    for _ in 0..QUEUED_EVENTS {
        assert_eq!(producer.emit(prepared), Emitted::Queued);
    }
    let barrier = Arc::new(Barrier::new(2));
    let (done, completion) = mpsc::sync_channel(1);
    let task = std::thread::spawn({
        let (producer, barrier) = (producer.clone(), barrier.clone());
        move || {
            barrier.wait();
            assert_eq!(producer.emit(prepared), Emitted::Full);
            done.send(()).expect("useful work progressed");
        }
    });
    barrier.wait();
    completion
        .recv_timeout(Duration::from_secs(2))
        .expect("producer cannot wait for consumer");
    task.join().expect("producer");
    assert_eq!(
        producer.loss.snapshot(),
        Loss {
            full: 1,
            ..Loss::default()
        }
    );
    let mut bytes = 0;
    for index in 1..=QUEUED_EVENTS {
        let event = receiver.try_recv().expect("bounded queue");
        bytes += event.len();
        assert_eq!(decode(&event).expect("event").sequence.get(), index as u64);
    }
    assert!(bytes <= QUEUED_EVENTS * MAX_EVENT_BYTES);
    assert!(receiver.try_recv().is_err());
    assert_eq!(producer.emit(prepared), Emitted::Queued);
    assert_eq!(
        decode(&receiver.try_recv().expect("after loss"))
            .expect("event")
            .sequence
            .get(),
        258
    );
}

#[test]
fn closure_exhaustion_and_counter_saturation_are_explicit() {
    let (producer, receiver) = pair();
    drop(receiver);
    assert_eq!(producer.emit(prepared), Emitted::Closed);
    assert_eq!(producer.loss.snapshot().closed, 1);
    producer.sequence.store(u64::MAX, Ordering::Relaxed);
    assert_eq!(
        producer.emit(|| panic!("exhausted producer must not prepare payload")),
        Emitted::Exhausted
    );
    assert_eq!(producer.sequence.load(Ordering::Relaxed), u64::MAX);
    assert_eq!(producer.loss.snapshot().exhausted, 1);
    producer.loss.exhausted.store(u64::MAX, Ordering::Relaxed);
    assert_eq!(producer.emit(prepared), Emitted::Exhausted);
    assert_eq!(producer.loss.snapshot().exhausted, u64::MAX);
}

#[test]
fn concurrent_producers_have_unique_sequences_and_account_for_loss() {
    let (producer, receiver) = pair();
    let producer = Arc::new(producer);
    let workers: Vec<_> = (0..4)
        .map(|_| {
            std::thread::spawn({
                let producer = producer.clone();
                move || {
                    for _ in 0..1000 {
                        assert!(matches!(
                            producer.emit(prepared),
                            Emitted::Queued | Emitted::Full
                        ));
                    }
                }
            })
        })
        .collect();
    for worker in workers {
        worker.join().expect("producer thread");
    }
    let mut sequences = HashSet::new();
    while let Ok(bytes) = receiver.try_recv() {
        assert!(sequences.insert(decode(&bytes).expect("event").sequence));
    }
    assert_eq!(sequences.len(), QUEUED_EVENTS);
    assert_eq!(producer.loss.snapshot().full, 4000 - QUEUED_EVENTS as u64);
    assert_eq!(producer.sequence.load(Ordering::Relaxed), 4001);
}

#[test]
fn local_identities_are_distinct_bounded_and_never_wrap() {
    let (producer, _) = pair();
    let producer = Arc::new(producer);
    let mut tasks = Vec::new();
    for _ in 0..4 {
        let producer = producer.clone();
        tasks.push(std::thread::spawn(move || {
            (0..100)
                .map(|_| String::from(producer.identity().expect("identity")))
                .collect::<Vec<_>>()
        }));
    }
    let ids: HashSet<_> = tasks
        .into_iter()
        .flat_map(|task| task.join().expect("producer joined"))
        .collect();
    assert_eq!(ids.len(), 400);
    assert_eq!(producer.sequence.load(Ordering::Relaxed), 1);
    assert_ne!(
        Id::local(identity(), 1),
        Id::local(Id::try_from("ff".repeat(16)).expect("producer"), 1)
    );
    producer.identities.store(u64::MAX - 1, Ordering::Relaxed);
    assert!(producer.identity().is_some());
    assert!(producer.identity().is_none());
    assert!(producer.identity().is_none());
    assert_eq!(producer.identities.load(Ordering::Relaxed), u64::MAX);
    assert_eq!(producer.loss.snapshot().exhausted, 2);
}

#[test]
fn typed_owner_facts_roundtrip_without_arbitrary_measurement_maps() {
    let (producer, receiver) = pair();
    for fact in [
        Fact::ProviderAttempt {
            outcome: AttemptOutcome::Dispatched,
            duration_ms: elapsed(Instant::now()),
        },
        Fact::Retry {
            outcome: RetryOutcome::Cancelled,
        },
        Fact::UsageSnapshot {
            revision: NonZeroU64::new(1).expect("revision"),
            input_tokens: None,
            output_tokens: None,
            cache_read_tokens: None,
            cache_write_tokens: None,
            reported_cost_micros: Some(7),
            estimated_cost_micros: None,
            ledger: LedgerOutcome::Failed,
        },
    ] {
        assert_eq!(
            producer.emit(|| (
                Correlation {
                    turn_id: Some(identity()),
                    ..Correlation::default()
                },
                fact
            )),
            Emitted::Queued
        );
        let event = decode(&receiver.try_recv().expect("event")).expect("schema");
        assert_eq!(event.fact, fact);
        assert_eq!(event.correlation.turn_id, Some(identity()));
    }
}
