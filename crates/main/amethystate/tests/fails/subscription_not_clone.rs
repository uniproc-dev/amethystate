use amethystate::{ReactiveScope, SignalSubscription};

fn assert_clone<T: Clone>() {}

fn main() {
    assert_clone::<SignalSubscription>();
    assert_clone::<ReactiveScope>();
}
