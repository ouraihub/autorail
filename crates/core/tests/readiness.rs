#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "integration test helpers fail loudly"
)]

use thinkrail_core::{
    BackendKind::{Dsh, Omp, Pi},
    Readiness, readiness,
};

#[test]
fn readiness_follows_the_default_and_what_is_installed() {
    assert_eq!(readiness(None, &[]), Readiness::NoneInstalled);
    assert_eq!(
        readiness(None, &[Dsh, Omp]),
        Readiness::NoDefault(Omp),
        "the first installed engine in ALL order"
    );
    assert_eq!(readiness(Some(Dsh), &[Pi, Dsh]), Readiness::Ready(Dsh));
    assert_eq!(
        readiness(Some(Omp), &[Pi]),
        Readiness::DefaultMissing(Omp),
        "another installed engine is not a fallback"
    );
    assert_eq!(readiness(Some(Pi), &[]), Readiness::DefaultMissing(Pi));
}
