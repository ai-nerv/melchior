use super::*;

#[test]
fn cancelling_every_native_model_after_partial_usage_closes_transport_and_reaps_the_ask() {
    let mut requests = 0;
    for (id, key, fallback, count) in PLANS {
        let (dir, listener) = fixture(id, key, fallback, count);
        for row in rows(id) {
            let (headers, body) =
                super::super::cancellation::after_usage(&dir, &listener, key, &request(id, &row));
            requests += 1;
            assert!(headers.starts_with("post /compatible-mode/v1/chat/completions "));
            assert!(headers.contains("authorization: bearer private-sentinel\r\n"));
            assert_eq!(body["model"], row["id"]);
            assert_eq!(body["max_completion_tokens"], 128);
        }
    }
    assert_eq!(requests, 66);
}
