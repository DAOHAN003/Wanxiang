#[test]
fn benchmark_target_smoke_test() {
    assert_eq!(std::hint::black_box(1 + 1), 2);
}
