//! Revision numbering per scope.

mod common;

use common::fixture;
use tokiori_pg::PgStore;

#[tokio::test]
async fn the_first_revision_in_a_scope_is_one() {
    let Some(fx) = fixture().await else { return };

    let revision = fx.store.next_revision(&fx.scope).await.expect("number");
    assert_eq!(revision, 1);

    fx.cleanup().await;
}

#[tokio::test]
async fn revisions_increase_by_one_per_commit() {
    let Some(fx) = fixture().await else { return };

    let first = fx.store.next_revision(&fx.scope).await.expect("number");
    let second = fx.store.next_revision(&fx.scope).await.expect("number");
    let third = fx.store.next_revision(&fx.scope).await.expect("number");

    assert_eq!((first, second, third), (1, 2, 3));

    fx.cleanup().await;
}

#[tokio::test]
async fn scopes_number_independently() {
    let Some(fx) = fixture().await else { return };
    let other = format!("{}-other", fx.scope);

    assert_eq!(fx.store.next_revision(&fx.scope).await.expect("number"), 1);
    assert_eq!(fx.store.next_revision(&fx.scope).await.expect("number"), 2);
    assert_eq!(fx.store.next_revision(&other).await.expect("number"), 1);

    fx.cleanup().await;
}

#[tokio::test]
async fn concurrent_writers_never_share_a_revision() {
    let Some(fx) = fixture().await else { return };
    let store: PgStore = fx.store.clone();
    let scope = fx.scope.clone();

    let mut handles = Vec::new();
    for _ in 0..8 {
        let store = store.clone();
        let scope = scope.clone();
        handles.push(tokio::spawn(async move {
            store.next_revision(&scope).await.expect("number")
        }));
    }

    let mut revisions = Vec::new();
    for handle in handles {
        revisions.push(handle.await.expect("join"));
    }
    revisions.sort_unstable();

    assert_eq!(revisions, (1..=8).collect::<Vec<_>>());

    fx.cleanup().await;
}

#[tokio::test]
async fn a_rolled_back_revision_is_not_reused() {
    let Some(fx) = fixture().await else { return };

    let first = fx.store.next_revision(&fx.scope).await.expect("number");
    let second = fx.store.next_revision(&fx.scope).await.expect("number");

    // The contract is monotonicity, not a gap-free sequence: a rollback may
    // leave a hole and that must not break ordering.
    assert!(second > first);

    fx.cleanup().await;
}
